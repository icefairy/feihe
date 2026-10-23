//! 词库二进制（`base.bin` / `user.bin`）的零解析读取器
//!
//! 同一个 `Bin` 读两种来源：
//!
//! - `base.bin`：构建期由 `build.rs` 生成，`include_bytes!` 嵌进 DLL 的 `.rdata`。
//!   这一段由 PE 加载器 mmap，**多个进程共享同一份物理页**。
//! - `user.bin`：运行期由 `dict::compile_user_bin` 生成，落在 `%APPDATA%\Feihe\`，
//!   启动时用 `map_file` 映射进来。同样是**文件映射，多进程共享页缓存**，
//!   所以用户词库不会退化成「每个进程各建一份堆上的索引」。
//!
//! 两者格式完全一致（同一个 `dictenc` 编码器），所以下面的查询代码通吃。
//!
//! 布局（全部小端）：
//!
//! ```text
//! 0   magic    "FHDB"
//! 4   version  u32
//! 8   count    u32
//! 12  reserved u32
//! 16  13 × (off: u32, len: u32)   各段在文件中的偏移与长度
//! 128 各段数据
//! ```
//!
//! 每个「池」配一张 `u32[count+1]` 的偏移表，第 i 条的字节区间是
//! `pool[off[i]..off[i+1]]`。编不出码的条目偏移表上就是空区间，自然被跳过。
//!
//! 查询 = 在「按码升序排的 raw 下标表」上二分，比较时按需去池里取码。
//! 没有解析、没有分配、没有排序。

use std::path::Path;

use crate::dictenc::{HEADER_LEN, MAGIC, NSEC, VERSION};

/// 段序号，必须和 `build.rs` 里的常量一致
const S_WORD_OFF: usize = 0;
const S_WORD_POOL: usize = 1;
const S_PY_OFF: usize = 2;
const S_PY_POOL: usize = 3;
const S_XH_OFF: usize = 4;
const S_XH_POOL: usize = 5;
const S_MS_OFF: usize = 6;
const S_MS_POOL: usize = 7;
const S_FLAT_OFF: usize = 8;
const S_FLAT_POOL: usize = 9;
const S_XH_IDX: usize = 10;
const S_MS_IDX: usize = 11;
const S_FLAT_IDX: usize = 12;

static RAW: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/base.bin"));

/// 查哪一套索引
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Idx {
    /// 小鹤双拼码
    Xiaohe,
    /// 微软双拼码
    Ms,
    /// 全拼去空格
    Flat,
}

impl Idx {
    fn off_sec(self) -> usize {
        match self {
            Idx::Xiaohe => S_XH_OFF,
            Idx::Ms => S_MS_OFF,
            Idx::Flat => S_FLAT_OFF,
        }
    }
    fn pool_sec(self) -> usize {
        match self {
            Idx::Xiaohe => S_XH_POOL,
            Idx::Ms => S_MS_POOL,
            Idx::Flat => S_FLAT_POOL,
        }
    }
    fn idx_sec(self) -> usize {
        match self {
            Idx::Xiaohe => S_XH_IDX,
            Idx::Ms => S_MS_IDX,
            Idx::Flat => S_FLAT_IDX,
        }
    }
}

pub struct Bin {
    data: &'static [u8],
    count: usize,
    sec: [(usize, usize); NSEC],
}

impl Bin {
    /// 一个查不到任何东西的词库。格式坏了时用它兜底——
    /// 输入法宁可打不出候选，也不能让宿主应用崩掉。
    fn empty(data: &'static [u8]) -> Self {
        Self {
            data,
            count: 0,
            sec: [(0, 0); NSEC],
        }
    }

    /// 校验头部并建视图。格式不对返回 `None`。
    ///
    /// `user.bin` 是运行期写出来的，可能被手改坏、也可能是旧版本格式，
    /// 所以调用方必须能区分「空词库」和「坏文件」——前者照用，后者要回退到 txt。
    fn try_new(data: &'static [u8]) -> Option<Self> {
        if data.len() < HEADER_LEN || &data[0..4] != MAGIC {
            return None;
        }
        let version = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        if version != VERSION {
            return None;
        }
        let count = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;
        let mut sec = [(0usize, 0usize); NSEC];
        for (i, slot) in sec.iter_mut().enumerate() {
            let b = 16 + i * 8;
            let off = u32::from_le_bytes([data[b], data[b + 1], data[b + 2], data[b + 3]]) as usize;
            let len =
                u32::from_le_bytes([data[b + 4], data[b + 5], data[b + 6], data[b + 7]]) as usize;
            if off + len > data.len() {
                return None;
            }
            *slot = (off, len);
        }
        Some(Self { data, count, sec })
    }

    fn u32_at(&self, pos: usize) -> u32 {
        let b = &self.data[pos..pos + 4];
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    }

    /// 第 i 条在某个池里的字节区间
    fn slice(&self, off_sec: usize, pool_sec: usize, i: usize) -> &'static str {
        let (ooff, _) = self.sec[off_sec];
        let start = self.u32_at(ooff + i * 4) as usize;
        let end = self.u32_at(ooff + (i + 1) * 4) as usize;
        let (poff, plen) = self.sec[pool_sec];
        let (s, e) = (poff + start, poff + end);
        if s > e || e > poff + plen {
            return "";
        }
        // 数据是我们自己生成的 ASCII/UTF-8，保证合法
        std::str::from_utf8(&self.data[s..e]).unwrap_or("")
    }

    pub fn count(&self) -> usize {
        self.count
    }

    /// 第 i 条的词面
    pub fn word(&self, i: usize) -> &'static str {
        self.slice(S_WORD_OFF, S_WORD_POOL, i)
    }

    /// 第 i 条的全拼原文（音节空格分隔）
    pub fn pinyin(&self, i: usize) -> &'static str {
        self.slice(S_PY_OFF, S_PY_POOL, i)
    }

    /// 第 i 条在某个方案下的码
    fn code(&self, which: Idx, i: usize) -> &'static str {
        self.slice(which.off_sec(), which.pool_sec(), i)
    }

    fn idx_len(&self, which: Idx) -> usize {
        self.sec[which.idx_sec()].1 / 4
    }

    /// 索引表第 p 位对应的 raw 下标。
    ///
    /// raw 下标就是**词库文件里的原始行号**，而 `dict.txt` 是按词频降序排的
    /// （第一行是「的」），所以它同时也是全局词频排名。前缀查询要跨多个音节，
    /// 光按码排序会让「按字母序最靠前的那个音节」包场（`d` 全是「大/打/答」），
    /// 必须拿这个下标把各音节的候选合并起来。
    pub fn raw_of(&self, which: Idx, p: usize) -> usize {
        let (off, _) = self.sec[which.idx_sec()];
        self.u32_at(off + p * 4) as usize
    }

    /// 前缀区间：返回索引表里 `[start, end)` 这一段，其码都以 `prefix` 开头。
    pub fn prefix_range(&self, which: Idx, prefix: &str) -> (usize, usize) {
        if prefix.is_empty() {
            return (0, 0);
        }
        let n = self.idx_len(which);
        let code_at = |p: usize| self.code(which, self.raw_of(which, p));

        // lower_bound：第一个 >= prefix
        let (mut lo, mut hi) = (0usize, n);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if code_at(mid) < prefix {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let start = lo;

        // upper_bound：从 start 起第一个不再以 prefix 开头
        let (mut lo, mut hi) = (start, n);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if code_at(mid).starts_with(prefix) {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        (start, lo)
    }

    /// 前缀排序键：`(码长, 原始行号)`。
    ///
    /// 一次取出，省掉重复读索引表——单键查询要扫几千条，每条少一次随机访问
    /// 就是实打实的收益（这个键的两部分本来就都来自同一条索引项）。
    pub fn rank_key(&self, which: Idx, p: usize) -> (usize, usize) {
        let i = self.raw_of(which, p);
        let (ooff, _) = self.sec[which.off_sec()];
        let start = self.u32_at(ooff + i * 4) as usize;
        let end = self.u32_at(ooff + (i + 1) * 4) as usize;
        (end.saturating_sub(start), i)
    }

    /// 索引表第 p 位对应的词面
    pub fn word_at(&self, which: Idx, p: usize) -> &'static str {
        self.word(self.raw_of(which, p))
    }

    /// 遍历前缀区间，产出词面。**不保证整码优先**，正式查询走 `dict::search`。
    #[allow(dead_code)]
    pub fn iter_prefix(&self, which: Idx, prefix: &str) -> impl Iterator<Item = &'static str> + '_ {
        let (s, e) = self.prefix_range(which, prefix);
        (s..e).map(move |p| self.word(self.raw_of(which, p)))
    }
}

pub fn get() -> &'static Bin {
    use std::sync::OnceLock;
    static B: OnceLock<Bin> = OnceLock::new();
    B.get_or_init(|| Bin::try_new(RAW).unwrap_or_else(|| Bin::empty(RAW)))
}

/// 把磁盘上的 FHDB 文件映射进地址空间。
///
/// 为什么是 `CreateFileMapping` 而不是 `fs::read`：
/// `fs::read` 会把整个文件拷进本进程的私有堆，每个宿主进程各一份，正是
/// `base.bin` 当初要消灭的东西。文件映射走的是**系统页缓存**，多个进程
/// 映射同一个文件时共享同一份物理页，第一次访问只付页错误。
///
/// 生命周期：视图和映射句柄**故意不释放**。返回的 `&'static Bin` 会被存进
/// 全局缓存，直到进程退出；进程退出时内核自动回收。这样既拿到了 `'static`
/// 又不用引入自引用的所有权结构。代价是重新编译 `user.bin` 后旧映射会残留
/// （每次重编译 +1 份共享页），而重编译只在设置窗里点按钮时才发生，可接受。
pub fn map_file(path: &Path) -> Option<&'static Bin> {
    use std::os::windows::io::AsRawHandle;

    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Memory::{
        CreateFileMappingW, MapViewOfFile, FILE_MAP_READ, PAGE_READONLY,
    };

    // std 打开文件时默认带 FILE_SHARE_READ|WRITE|DELETE，
    // 所以别的进程能把 user.bin 改名替换掉（重编译要用到这个性质）。
    let file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len() as usize;
    if len < HEADER_LEN {
        return None;
    }

    let mapping = unsafe {
        CreateFileMappingW(
            HANDLE(file.as_raw_handle() as _),
            None,
            PAGE_READONLY,
            0,
            0,
            windows::core::PCWSTR::null(),
        )
    }
    .ok()?;

    // 映射建立后自己就持有文件引用，可以把 File 关掉
    drop(file);

    let view = unsafe { MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, 0) };
    let ptr = view.Value as *const u8;
    if ptr.is_null() {
        return None;
    }

    // SAFETY: 视图覆盖整个文件（dwNumberOfBytesToMap=0），文件在映射期间不会被
    // 我们改写；映射永不解除，所以这个切片在进程生命周期内始终有效且只读。
    let data: &'static [u8] = unsafe { std::slice::from_raw_parts(ptr, len) };
    let bin = Bin::try_new(data)?;
    Some(Box::leak(Box::new(bin)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_ok() {
        let b = get();
        assert!(b.count() > 1000, "count = {}", b.count());
    }

    #[test]
    fn xiaohe_prefix() {
        // nihc = ni hao（小鹤）
        let hits: Vec<_> = get().iter_prefix(Idx::Xiaohe, "nihc").take(50).collect();
        assert!(hits.iter().any(|w| *w == "你好"), "got {hits:?}");
    }

    #[test]
    fn retroflex() {
        // 小鹤 shi = ui、zhong = vs
        let ui: Vec<_> = get().iter_prefix(Idx::Xiaohe, "ui").take(50).collect();
        assert!(ui.iter().any(|w| *w == "是"), "got {ui:?}");
        let vs: Vec<_> = get().iter_prefix(Idx::Xiaohe, "vs").take(50).collect();
        assert!(vs.iter().any(|w| *w == "中"), "got {vs:?}");
    }

    #[test]
    fn ms_prefix() {
        // 微软：ing 在分号上，ying = y;
        let hits: Vec<_> = get().iter_prefix(Idx::Ms, "y;").take(50).collect();
        assert!(!hits.is_empty(), "y; should match something");
    }

    #[test]
    fn flat_prefix() {
        let hits: Vec<_> = get().iter_prefix(Idx::Flat, "nihao").take(50).collect();
        assert!(hits.iter().any(|w| *w == "你好"), "got {hits:?}");
    }

    #[test]
    fn word_and_pinyin_align() {
        let b = get();
        let hits: Vec<_> = b.iter_prefix(Idx::Flat, "nihao").take(5).collect();
        assert!(!hits.is_empty());
        // 随便取一条验证 pinyin 能读出来
        let (s, _) = b.prefix_range(Idx::Flat, "nihao");
        let raw = b.raw_of(Idx::Flat, s);
        assert!(!b.pinyin(raw).is_empty());
    }

    #[test]
    fn empty_prefix() {
        assert_eq!(get().prefix_range(Idx::Xiaohe, ""), (0, 0));
    }

    /// 运行期生成的 `user.bin` 走的是「写文件 → mmap → 同一套查询」这条路，
    /// 和 `base.bin` 的 `include_bytes!` 完全不同，必须单独钉住。
    #[test]
    fn map_file_roundtrip() {
        let entries = crate::dictenc::parse_entries("飞鹤输入法\tfei he shu ru fa\n你好\tni hao\n");
        let built = crate::dictenc::build(&entries);

        let dir = std::env::temp_dir().join("feihe_map_roundtrip");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("user.bin");
        std::fs::write(&p, &built.bytes).unwrap();

        let b = map_file(&p).expect("映射应该成功");
        assert_eq!(b.count(), 2);
        let (s, e) = b.prefix_range(Idx::Flat, "feihe");
        assert!(s < e, "全拼 feihe 应该有命中");
        let hits: Vec<_> = (s..e).map(|i| b.word_at(Idx::Flat, i)).collect();
        assert!(hits.contains(&"飞鹤输入法"), "got {hits:?}");
    }

    #[test]
    fn map_file_rejects_garbage() {
        let dir = std::env::temp_dir().join("feihe_map_garbage");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("bad.bin");
        std::fs::write(&p, vec![0u8; 512]).unwrap();
        assert!(map_file(&p).is_none(), "魔数不对必须返回 None");
    }

    /// 重编译用户词库时，要把新文件**改名覆盖**一个正被本进程映射的旧文件。
    /// Windows 上「文件正在使用」通常挡掉删除/覆盖，这条必须钉住，
    /// 否则 `compile_user_bin` 会在第二次编译时莫名其妙失败。
    #[test]
    fn rename_over_mapped_file() {
        let dir = std::env::temp_dir().join("feihe_rename_mapped");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("user.bin");

        let first = crate::dictenc::build(&crate::dictenc::parse_entries("你好\tni hao\n"));
        std::fs::write(&p, &first.bytes).unwrap();
        let old = map_file(&p).expect("首次映射");
        assert_eq!(old.count(), 1);

        let second = crate::dictenc::build(&crate::dictenc::parse_entries(
            "你好\tni hao\n飞鹤\tfei he\n",
        ));
        let tmp = dir.join("user.bin.tmp");
        std::fs::write(&tmp, &second.bytes).unwrap();
        std::fs::rename(&tmp, &p).expect("改名覆盖被映射的文件应该成功");

        let new = map_file(&p).expect("重新映射");
        assert_eq!(new.count(), 2, "新映射应看到新内容");
        assert_eq!(old.count(), 1, "旧映射仍指向旧文件（Windows 语义）");
    }

    /// 对比「5 万条用户词库」的两种加载方式，给「编译成 bin 值不值得」一个实数。
    ///
    /// `cargo test --release -- --ignored user_bin_vs_overlay --nocapture`
    #[test]
    #[ignore = "手工测量用，依赖 src/user_dict.txt"]
    fn user_bin_vs_overlay() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/user_dict.txt");
        let Ok(text) = std::fs::read_to_string(&src) else {
            return;
        };

        // ① 一次性：编译（设置窗里点「编译成二进制」时才发生）
        let t = std::time::Instant::now();
        let entries = crate::dictenc::parse_entries(&text);
        let built = crate::dictenc::build(&entries);
        let compile_ms = t.elapsed().as_secs_f64() * 1000.0;

        let dir = std::env::temp_dir().join("feihe_bench_userbin");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("user.bin");
        std::fs::write(&p, &built.bytes).unwrap();

        // ② 每进程：映射 + 首次查询（这才是常态开销）
        let t = std::time::Instant::now();
        let b = map_file(&p).expect("映射失败");
        let (s, e) = b.prefix_range(Idx::Xiaohe, "nihc");
        let n = e - s;
        let map_ms = t.elapsed().as_secs_f64() * 1000.0;

        println!(
            "user.bin: {} 条 / {} KB\n  编译（一次性）{compile_ms:.1} ms\n  映射+首查（每进程）{map_ms:.3} ms（命中 {n}）",
            built.count,
            built.bytes.len() / 1024
        );
    }
}
