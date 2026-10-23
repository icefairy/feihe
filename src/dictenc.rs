//! FHDB 词库编码器：`词<TAB>全拼` 文本 → 扁平二进制
//!
//! 这个模块被**两个地方**以 `#[path]` 方式引入，共用同一份实现：
//!
//! - `build.rs`：构建期生成 `base.bin`，`include_bytes!` 进 DLL 的 `.rdata`
//! - `dict::compile_user_bin`：运行期生成 `%APPDATA%\Feihe\user.bin`，运行时 mmap
//!
//! 两边用同一个编码器，格式天然一致，`dictbin::Bin` 一套零解析读取代码通吃。
//! 如果各写一份，改了布局忘了改另一边，用户词库就会静默读不出来——这类 bug
//! 极难查（表现是「导入的词不生效」），所以这里刻意不重复实现。
//!
//! 布局（全部小端，与 `src/dictbin.rs` 的常量必须一致）：
//!
//! ```text
//! 0   "FHDB" | version u32 | count u32 | reserved u32
//! 16  13 × (off u32, len u32)    各段偏移与长度
//! 128 各段：5 张偏移表(u32[count+1]) + 3 张索引表(u32[]) + 5 个字节池
//! ```

use std::collections::HashSet;

use crate::shuangpin::{encode_pinyin, Scheme};

/// 段数，必须和 `dictbin.rs` 里的常量一致
pub const NSEC: usize = 13;
pub const HEADER_LEN: usize = 128;
pub const MAGIC: &[u8; 4] = b"FHDB";
pub const VERSION: u32 = 1;

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

/// 编码结果
///
/// `xh` / `ms` / `flat` 只有 `build.rs` 拿来打日志；运行期编译 user.bin 时用不到，
/// 但共用同一份实现，所以这里允许它们「在 lib 侧无人读取」。
#[allow(dead_code)]
pub struct Built {
    pub bytes: Vec<u8>,
    /// 总条目数
    pub count: usize,
    /// 各方案下能编出码的条数（编不出码的条目不进索引，等于查不到）
    pub xh: usize,
    pub ms: usize,
    pub flat: usize,
}

/// 解析 `词<TAB>全拼` 文本。按出现顺序去重，先出现的赢。
///
/// 空行、`#` 开头的注释行、没有 TAB 的行直接跳过——用户手改文本时很容易带进这些。
pub fn parse_entries(text: &str) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    push_entries(text, &mut seen, &mut out);
    out
}

/// 把一份文本的条目并进 `out`，`seen` 跨文件共享以做到全局去重。
pub fn push_entries(text: &str, seen: &mut HashSet<String>, out: &mut Vec<(String, String)>) {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
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
        out.push((w.to_string(), py.to_string()));
    }
}

/// 把条目编码成 FHDB v1 二进制。
pub fn build(entries: &[(String, String)]) -> Built {
    let n = entries.len();

    // 五组「偏移表 + 字节池」：词面、全拼原文、小鹤码、微软码、全拼去空格
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

    for (w, py) in entries {
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

    let mut buf: Vec<u8> = Vec::with_capacity(HEADER_LEN + n * 48);
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
    buf[0..4].copy_from_slice(MAGIC);
    buf[4..8].copy_from_slice(&VERSION.to_le_bytes());
    buf[8..12].copy_from_slice(&(n as u32).to_le_bytes());
    buf[12..16].copy_from_slice(&0u32.to_le_bytes());
    for i in 0..NSEC {
        let base = 16 + i * 8;
        buf[base..base + 4].copy_from_slice(&sec_off[i].to_le_bytes());
        buf[base + 4..base + 8].copy_from_slice(&sec_len[i].to_le_bytes());
    }

    Built {
        bytes: buf,
        count: n,
        xh: xh_idx.len(),
        ms: ms_idx.len(),
        flat: flat_idx.len(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_header() {
        let entries = parse_entries("# 注释\n你好\tni hao\n飞鹤\tfei he\n坏行没有tab\n\n");
        assert_eq!(entries.len(), 2);
        let b = build(&entries);
        assert_eq!(&b.bytes[0..4], MAGIC);
        assert_eq!(b.count, 2);
        assert_eq!(b.xh, 2, "两个词都该能编出小鹤码");
    }

    #[test]
    fn dedup_keeps_first() {
        let entries = parse_entries("你好\tni hao\n你好\tha o\n");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].1, "ni hao");
    }

    #[test]
    fn unencodable_still_counts_but_not_indexed() {
        // 「嗯」的拼音 ng 在小鹤里没有合法韵母，编不出码：条目保留，索引里没有
        let b = build(&parse_entries("嗯\tng\n你好\tni hao\n"));
        assert_eq!(b.count, 2);
        assert_eq!(b.xh, 1);
    }
}
