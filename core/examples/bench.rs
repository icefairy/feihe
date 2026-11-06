//! 核心引擎的性能基准。`cargo run --release --example bench -p feihe-core`
//!
//! **要和发布包对齐就带上 `FEIHE_NO_USER_DICT=1`**：否则会把本机个人词库并进
//! base.bin（46k → 98k 条），查词延迟会被放大一倍多，数字不能拿去和线上比。
//!
//! 用计数分配器同时统计每次调用的**堆分配次数/字节**——输入法每次按键都在 UI 线程
//! 上跑这些函数，延迟之外，分配抖动才是最容易被忽略的那一项。

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use feihe_core::{dict, next, paths};

struct Counting;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(l.size(), Ordering::Relaxed);
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(n.saturating_sub(l.size()), Ordering::Relaxed);
        System.realloc(p, l, n)
    }
}

#[global_allocator]
static A: Counting = Counting;

fn stats(mut v: Vec<u64>) -> (f64, f64, f64) {
    v.sort_unstable();
    let p = |q: f64| {
        let i = ((v.len() as f64 - 1.0) * q).round() as usize;
        v[i] as f64 / 1000.0
    };
    (p(0.5), p(0.95), p(0.99))
}

fn bench<F: FnMut()>(name: &str, n: usize, mut f: F) {
    for _ in 0..(n / 10).min(2000) {
        f();
    }
    let mut d = Vec::with_capacity(n);
    let a0 = ALLOCS.load(Ordering::Relaxed);
    let b0 = BYTES.load(Ordering::Relaxed);
    for _ in 0..n {
        let t = Instant::now();
        f();
        d.push(t.elapsed().as_nanos() as u64);
    }
    let a1 = ALLOCS.load(Ordering::Relaxed);
    let b1 = BYTES.load(Ordering::Relaxed);
    let (p50, p95, p99) = stats(d);
    println!(
        "{:<34} p50 {:>9.1}us  p95 {:>9.1}us  p99 {:>9.1}us   {:>5.1} allocs/call {:>7.0} B/call",
        name,
        p50,
        p95,
        p99,
        (a1 - a0) as f64 / n as f64,
        (b1 - b0) as f64 / n as f64,
    );
}

fn main() {
    let dir = std::env::temp_dir().join("feihe-bench");
    let _ = std::fs::create_dir_all(&dir);
    paths::init(dir);
    feihe_core::dict::set_scheme(feihe_core::shuangpin::Scheme::Xiaohe);

    // ── 冷启动 ────────────────────────────────────────────────
    let t = Instant::now();
    let first = dict::lookup_input("nihao", true, 9);
    println!("冷启动 首次 lookup_input: {:.3} ms -> {:?}", t.elapsed().as_secs_f64() * 1e3, &first.words[..first.words.len().min(3)]);
    let (a0, b0) = (ALLOCS.load(Ordering::Relaxed), BYTES.load(Ordering::Relaxed));
    let t = Instant::now();
    let s = next::suggest("北京", 5);
    let ms = t.elapsed().as_secs_f64() * 1e3;
    let (a1, b1) = (ALLOCS.load(Ordering::Relaxed), BYTES.load(Ordering::Relaxed));
    println!(
        "冷启动 首次 suggest(北京): {:.3} ms -> {:?}  解析分配 {} 次 / {} KB",
        ms,
        s,
        a1 - a0,
        (b1 - b0) / 1024
    );
    let t = Instant::now();
    let s = next::suggest("人工智能", 5);
    println!("冷启动 首次 suggest(人工智能, 后缀回退): {:.3} ms -> {:?}", t.elapsed().as_secs_f64() * 1e3, s);

    println!();
    println!("== 每次按键：lookup_input（Android 每敲一个字母就跑一次）==");
    bench("lookup 1 键 'n'", 20000, || {
        let _ = dict::lookup_input("n", true, 9);
    });
    bench("lookup 半截 'nih'", 20000, || {
        let _ = dict::lookup_input("nih", true, 9);
    });
    bench("lookup 全拼 'nihao'", 20000, || {
        let _ = dict::lookup_input("nihao", true, 9);
    });
    bench("lookup 双拼 'nihc'", 20000, || {
        let _ = dict::lookup_input("nihc", false, 9);
    });
    bench("lookup 长串 'zhongguorenmin'", 20000, || {
        let _ = dict::lookup_input("zhongguorenmin", true, 9);
    });
    bench("lookup 长串分段 'vhsjli'", 20000, || {
        let _ = dict::lookup_input("vhsjli", false, 9);
    });
    bench("lookup 乱码 'zzzzzzzz'（纠错路径）", 20000, || {
        let _ = dict::lookup_input("zzzzzzzz", false, 9);
    });
    bench("lookup 乱码 16 键（纠错最坏）", 5000, || {
        let _ = dict::lookup_input("zzzzzzzzzzzzzzzz", false, 9);
    });
    bench("lookup 超长串 24 键 'vhsjli…'", 5000, || {
        let _ = dict::lookup_input("vhsjlivhsjlivhsjli", false, 9);
    });
    bench("lookup 全拼长句 22 键", 5000, || {
        let _ = dict::lookup_input("zhongguorenminjianshe", true, 9);
    });

    println!();
    println!("== 上屏后：suggest / record ==");
    bench("suggest 精确命中 '北京'", 20000, || {
        let _ = next::suggest("北京", 5);
    });
    bench("suggest 后缀回退 '人工智能'", 20000, || {
        let _ = next::suggest("人工智能", 5);
    });
    bench("suggest 未命中 '一二三四五'", 20000, || {
        let _ = next::suggest("一二三四五", 5);
    });
    bench("record '北京'->'时间'", 20000, || {
        next::record("北京", "时间");
    });

    println!();
    println!("== 逐段选择 ==");
    bench("segment_candidates('vhsjli')", 5000, || {
        let _ = dict::segment_candidates("vhsjli", 9);
    });
    bench("segment_candidates(24 键)", 2000, || {
        let _ = dict::segment_candidates("vhsjlivhsjlivhsjli", 9);
    });
    bench("compose_sentence(24 键)", 2000, || {
        let _ = dict::compose_sentence("vhsjlivhsjlivhsjli");
    });

    println!();
    println!(
        "内置表: {} 头词；用户表: {} 对；字典: {} 条 / {} 单字 / {:?}",
        next::builtin_count(),
        next::count(),
        dict::stats().0,
        dict::stats().1,
        dict::stats().2
    );
}
