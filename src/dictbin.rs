//! 词库二进制（`base.bin`）的零解析读取器
//!
//! 文件由 `build.rs` 生成，通过 `include_bytes!` 嵌进 DLL 的 `.rdata` 段。
//! 这一段由 PE 加载器 mmap，**多个进程共享同一份物理页**，所以词库的常驻内存
//! 从「每进程 9.2MB 私有堆」变成「全系统一份只读页，按需调入」。
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

const NSEC: usize = 13;
const HEADER_LEN: usize = 128;

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
    /// 解析头部。格式不对时返回一个空词库而不是 panic——
    /// 输入法宁可打不出候选，也不能让宿主应用崩掉。
    fn parse(data: &'static [u8]) -> Self {
        let bad = Self {
            data,
            count: 0,
            sec: [(0, 0); NSEC],
        };
        if data.len() < HEADER_LEN || &data[0..4] != b"FHDB" {
            return bad;
        }
        let version = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        if version != 1 {
            return bad;
        }
        let count = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;
        let mut sec = [(0usize, 0usize); NSEC];
        for (i, slot) in sec.iter_mut().enumerate() {
            let b = 16 + i * 8;
            let off = u32::from_le_bytes([data[b], data[b + 1], data[b + 2], data[b + 3]]) as usize;
            let len =
                u32::from_le_bytes([data[b + 4], data[b + 5], data[b + 6], data[b + 7]]) as usize;
            if off + len > data.len() {
                return bad;
            }
            *slot = (off, len);
        }
        Self { data, count, sec }
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

    /// 索引表第 p 位对应的 raw 下标
    fn raw_of(&self, which: Idx, p: usize) -> usize {
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

    /// 「码恰好等于 `code`」这一段的上界。
    ///
    /// 索引按码升序，所以整码匹配一定是从 `prefix_range().0` 开始的一段连续区间，
    /// 到 `exact_end` 结束。用它把整码候选排在更长的词前面——
    /// 否则打 `iuliqi`（处理器）会被 `处理器吧` 这种更长的词抢走第一位。
    pub fn exact_end(&self, which: Idx, code: &str) -> usize {
        let n = self.idx_len(which);
        let code_at = |p: usize| self.code(which, self.raw_of(which, p));
        let (mut lo, mut hi) = (0usize, n);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if code_at(mid) <= code {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
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
    B.get_or_init(|| Bin::parse(RAW))
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
}
