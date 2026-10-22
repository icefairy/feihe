// 构建脚本，做两件事：
//
// 1. 生成词库二进制 `$OUT_DIR/base.bin`（见下方 `gen_dict_bin`）
// 2. 用 windres 编译 installer/feihe.rc -> .o，作为链接参数把图标塞进 DLL
use std::path::PathBuf;
use std::process::Command;

// 直接复用运行时的编码器，避免双拼逻辑写两份、改一处忘一处。
// `src/shuangpin.rs` 只依赖 std，可以原样拉进 build script 编译。
#[allow(dead_code)]
#[path = "src/shuangpin.rs"]
mod shuangpin;

use shuangpin::{encode_pinyin, Scheme};

/// 段数，必须和 `src/dictbin.rs` 里的常量保持一致
const NSEC: usize = 13;

// 段序号（与 dictbin.rs 对齐）
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

const HEADER_LEN: usize = 128;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());

    gen_dict_bin(&manifest, &out);

    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() != "windows" {
        return;
    }

    let inc = manifest.join("installer");
    let obj = out.join("feihe-res.o");

    println!("cargo:rerun-if-changed=installer/feihe.rc");
    println!("cargo:rerun-if-changed=installer/resource.h");
    println!("cargo:rerun-if-changed=installer/assets/feihe.ico");

    let rc = inc.join("feihe.rc");
    let inc_s = inc.to_string_lossy().to_string();
    let obj_s = obj.to_string_lossy().to_string();

    // 优先 WINDRES 环境变量；否则按目标架构选 llvm-mingw 的 windres，再退回通用 windres。
    //
    // 注意：windres 产出的 COFF 目标文件带机器类型，x86_64 的 .o 塞进 i686 链接会报
    // `machine type x64 conflicts with x86`，所以这里必须跟着 target 走，
    // 不能像早期版本那样写死 x86_64。
    let triple = match std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default().as_str() {
        "x86" => "i686-w64-mingw32",
        "aarch64" => "aarch64-w64-mingw32",
        _ => "x86_64-w64-mingw32",
    };
    let mut candidates = Vec::new();
    if let Ok(v) = std::env::var("WINDRES") {
        candidates.push(v);
    }
    candidates.push(format!("{triple}-windres"));
    candidates.push("windres".into());

    for wr in &candidates {
        let ok = Command::new(wr)
            .current_dir(&inc)
            .arg(&rc)
            .arg("-I").arg(&inc_s)
            .arg("-O").arg("coff")
            .arg("-o").arg(&obj_s)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            println!("cargo:rustc-link-arg={}", obj_s);
            return;
        }
    }
    println!("cargo:warning=windres 未找到或编译失败，跳过图标资源（IconFile 将无图标）");
}

// ─────────────────────────────────────────────────────────────
// 词库二进制生成
// ─────────────────────────────────────────────────────────────

/// 把 `src/user_dict.txt` + `src/dict.txt` 编译成扁平二进制。
///
/// 为什么不用 `include_str!` + 运行时编码：
/// - 运行时要把 8 万条全拼音节编码成双拼码并排序，`Activate` 里要付 81ms，**每个进程各付一次**
/// - 编码后是堆上的一堆 `Arc`/`Box`，约 9.2MB **私有**内存，同样是每个进程各一份
///
/// 预编码之后，三套码都在构建期算好，运行时只是「读两个 u32 数组 + 二分」，
/// 而 `include_bytes!` 的数据落在 DLL 的 `.rdata` 段里——这一段由 PE 加载器 mmap，
/// **多个进程共享同一份物理页**，实际触达的页数只跟查询次数有关。
fn gen_dict_bin(manifest: &std::path::Path, out: &std::path::Path) {
    // 优先级：用户词在前（同码时排在前面，实现「用户词优先」），
    // 然后是基础词库，最后是人工补充的常用词（只含前两份里缺的词）。
    //
    // `FEIHE_NO_USER_DICT=1` 时跳过 user_dict.txt。
    // 打包分发必须用它：那份文件是从本机搜狗导出的个人打字记录，
    // 一旦并进 base.bin 就随 DLL 一起发出去了，而且是从二进制里抠不出来的。
    let skip_user = std::env::var("FEIHE_NO_USER_DICT").ok().as_deref() == Some("1");
    println!("cargo:rerun-if-env-changed=FEIHE_NO_USER_DICT");

    let mut sources: Vec<&str> = Vec::new();
    if skip_user {
        println!("cargo:warning=FEIHE_NO_USER_DICT=1：已排除 src/user_dict.txt（发布构建）");
    } else {
        sources.push("src/user_dict.txt");
    }
    sources.push("src/dict.txt");
    sources.push("src/extra_dict.txt");

    let mut entries: Vec<(String, String)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for f in sources {
        println!("cargo:rerun-if-changed={f}");
        let Ok(text) = std::fs::read_to_string(manifest.join(f)) else {
            continue;
        };
        for line in text.lines() {
            let Some((w, py)) = line.split_once('\t') else {
                continue;
            };
            let (w, py) = (w.trim(), py.trim());
            if w.is_empty() || py.is_empty() {
                continue;
            }
            if !seen.insert(w.to_string()) {
                continue;
            }
            entries.push((w.to_string(), py.to_string()));
        }
    }
    if entries.is_empty() {
        println!("cargo:warning=词库为空，base.bin 只会有头部");
    }

    let n = entries.len();

    // 四组「偏移表 + 字节池」：词面、全拼原文、小鹤码、微软码、全拼去空格
    let mut word_off = Vec::with_capacity(n + 1);
    let mut word_pool: Vec<u8> = Vec::with_capacity(n * 4);
    let mut py_off = Vec::with_capacity(n + 1);
    let mut py_pool: Vec<u8> = Vec::with_capacity(n * 10);
    let mut xh_off = Vec::with_capacity(n + 1);
    let mut xh_pool: Vec<u8> = Vec::with_capacity(n * 6);
    let mut ms_off = Vec::with_capacity(n + 1);
    let mut ms_pool: Vec<u8> = Vec::with_capacity(n * 6);
    let mut flat_off = Vec::with_capacity(n + 1);
    let mut flat_pool: Vec<u8> = Vec::with_capacity(n * 10);

    for (w, py) in &entries {
        word_off.push(word_pool.len() as u32);
        word_pool.extend_from_slice(w.as_bytes());

        py_off.push(py_pool.len() as u32);
        py_pool.extend_from_slice(py.as_bytes());

        match encode_pinyin(Scheme::Xiaohe, py) {
            Some(c) => {
                xh_off.push(xh_pool.len() as u32);
                xh_pool.extend_from_slice(c.as_bytes());
            }
            None => xh_off.push(xh_pool.len() as u32),
        }
        match encode_pinyin(Scheme::Ms, py) {
            Some(c) => {
                ms_off.push(ms_pool.len() as u32);
                ms_pool.extend_from_slice(c.as_bytes());
            }
            None => ms_off.push(ms_pool.len() as u32),
        }
        let flat: String = py.chars().filter(|c| !c.is_whitespace()).collect();
        flat_off.push(flat_pool.len() as u32);
        flat_pool.extend_from_slice(flat.as_bytes());
    }
    // 哨兵：最后一项指向池末尾
    word_off.push(word_pool.len() as u32);
    py_off.push(py_pool.len() as u32);
    xh_off.push(xh_pool.len() as u32);
    ms_off.push(ms_pool.len() as u32);
    flat_off.push(flat_pool.len() as u32);

    // 三套索引：按码升序排的 raw 下标。编不出码的条目（ng/hm 之类）不进索引。
    let xh_idx = make_index(&xh_pool, &xh_off, n);
    let ms_idx = make_index(&ms_pool, &ms_off, n);
    let flat_idx = make_index(&flat_pool, &flat_off, n);

    let mut buf: Vec<u8> = Vec::with_capacity(HEADER_LEN + 7_000_000);
    buf.resize(HEADER_LEN, 0);
    let mut sec_off = [0u32; NSEC];
    let mut sec_len = [0u32; NSEC];

    let mut push = |idx: usize, bytes: &[u8], buf: &mut Vec<u8>| {
        while buf.len() % 4 != 0 {
            buf.push(0);
        }
        sec_off[idx] = buf.len() as u32;
        sec_len[idx] = bytes.len() as u32;
        buf.extend_from_slice(bytes);
    };

    let u32s = |v: &[u32]| -> Vec<u8> {
        let mut b = Vec::with_capacity(v.len() * 4);
        for x in v {
            b.extend_from_slice(&x.to_le_bytes());
        }
        b
    };

    // 顺序无所谓（头部记了偏移），但把 u32 数组放前面、字节池放后面更整齐
    push(S_WORD_OFF, &u32s(&word_off), &mut buf);
    push(S_PY_OFF, &u32s(&py_off), &mut buf);
    push(S_XH_OFF, &u32s(&xh_off), &mut buf);
    push(S_MS_OFF, &u32s(&ms_off), &mut buf);
    push(S_FLAT_OFF, &u32s(&flat_off), &mut buf);
    push(S_XH_IDX, &u32s(&xh_idx), &mut buf);
    push(S_MS_IDX, &u32s(&ms_idx), &mut buf);
    push(S_FLAT_IDX, &u32s(&flat_idx), &mut buf);
    push(S_WORD_POOL, &word_pool, &mut buf);
    push(S_PY_POOL, &py_pool, &mut buf);
    push(S_XH_POOL, &xh_pool, &mut buf);
    push(S_MS_POOL, &ms_pool, &mut buf);
    push(S_FLAT_POOL, &flat_pool, &mut buf);

    // 头部：magic | version | count | reserved | 13×(off,len)
    buf[0..4].copy_from_slice(b"FHDB");
    buf[4..8].copy_from_slice(&1u32.to_le_bytes());
    buf[8..12].copy_from_slice(&(n as u32).to_le_bytes());
    buf[12..16].copy_from_slice(&0u32.to_le_bytes());
    for i in 0..NSEC {
        let base = 16 + i * 8;
        buf[base..base + 4].copy_from_slice(&sec_off[i].to_le_bytes());
        buf[base + 4..base + 8].copy_from_slice(&sec_len[i].to_le_bytes());
    }

    let path = out.join("base.bin");
    std::fs::write(&path, &buf).expect("写 base.bin 失败");
    println!(
        "cargo:warning=base.bin: {n} 条, {} KB (小鹤 {} / 微软 {} 可编码)",
        buf.len() / 1024,
        xh_idx.len(),
        ms_idx.len()
    );
}

/// 生成按码升序的 raw 下标表。稳定排序 → 同码内保持输入顺序（用户词在前、高频在前）。
fn make_index(pool: &[u8], off: &[u32], n: usize) -> Vec<u32> {
    let mut v: Vec<(u32, &[u8])> = (0..n)
        .filter_map(|i| {
            let s = &pool[off[i] as usize..off[i + 1] as usize];
            if s.is_empty() {
                None
            } else {
                Some((i as u32, s))
            }
        })
        .collect();
    v.sort_by(|a, b| a.1.cmp(b.1));
    v.into_iter().map(|(i, _)| i).collect()
}
