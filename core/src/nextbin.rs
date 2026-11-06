//! 内置联想表的**零解析**读取器（next.bin）。
//!
//! ## 为什么不再用 include_str! + 运行时解析
//!
//! 原来是 309 KB 文本、第一次 suggest 才 parse_builtin：实测 **5.8 ms、3.7 万次分配、
//! 2.1 MB 堆**。那 5.8 ms 落在「第一次上屏」或「键盘带着已有文字弹出」的瞬间，是用户
//! 能直接感觉到的一下；2.1 MB 还是**每个宿主进程一份**（Windows 上 explorer / 微信 /
//! Word 各一份）。
//!
//! 改成构建期编好的扁平二进制之后：
//!
//! - 第一次查询就是一次二分，没有解析、没有分配、没有堆；
//! - include_bytes! 的数据落在 DLL/.so 的只读段，由加载器 mmap，**多进程共享同一份
//!   物理页**，和 base.bin 是同一套思路（见 dictbin 的模块注释）。
//!
//! 和 dictbin 的区别：词库要按「码」查、有三套索引；联想表只需要按「头词」查一次，
//! 所以是一张排序表 + 两个池，格式简单到能一眼看完。
//!
//! ## 布局（全部小端）
//!
//! 0   magic    "FHNB"
//! 4   version  u32        （改格式就 +1，读到不认识的就当空表）
//! 8   heads    u32        头词个数 n
//! 12  pairs    u32        词对总数
//! 16  head_off u32[n+1]   第 i 个头词在 head_pool 里的字节区间
//!     tail_off u32[n+1]   第 i 个头词的后续在 tails 里的下标区间
//!     head_pool          头词字节，按 UTF-8 字节序升序（二分靠它）
//!     tails    u32[pairs] 每条后续词在 tail_pool 里的偏移
//!     tail_pool           [len: u8][词字节…]
//!
//! **不存权重**：内置表的排序在生成时就定好了（gen_next.py 按词库词频排），运行时
//! 只需要「按顺序取」，从来没有跨表比过数值。存 u32 权重会让文件多出 80 KB，换不来
//! 任何行为。顺序的回归由 next.rs 的 builtin_tail_order_is_preserved 守着。

use std::str;

const MAGIC: &[u8; 4] = b"FHNB";
const VERSION: u32 = 1;
const HEADER: usize = 16;

fn u32_at(d: &[u8], off: usize) -> u32 {
    let b = &d[off..off + 4];
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// 一张只读的联想表。数据来自 include_bytes!，生命周期是 'static。
pub struct Bin {
    d: &'static [u8],
}

impl Bin {
    pub const fn new(d: &'static [u8]) -> Self {
        Bin { d }
    }

    fn u32_at(&self, off: usize) -> u32 {
        u32_at(self.d, off)
    }

    /// 数据不完整 / 版本不认识时一律当空表——联想是锦上添花，不能因为它让输入法起不来。
    fn valid(&self) -> bool {
        self.d.len() >= HEADER && &self.d[0..4] == MAGIC && self.u32_at(4) == VERSION
    }

    pub fn head_count(&self) -> usize {
        if self.valid() { self.u32_at(8) as usize } else { 0 }
    }

    pub fn pair_count(&self) -> usize {
        if self.valid() { self.u32_at(12) as usize } else { 0 }
    }

    fn head_off_pos(&self) -> usize {
        HEADER
    }

    fn tail_off_pos(&self) -> usize {
        HEADER + 4 * (self.head_count() + 1)
    }

    fn heads_pos(&self) -> usize {
        self.tail_off_pos() + 4 * (self.head_count() + 1)
    }

    fn tails_pos(&self) -> usize {
        self.heads_pos() + self.u32_at(self.head_off_pos() + 4 * self.head_count()) as usize
    }

    fn tail_pool_pos(&self) -> usize {
        self.tails_pos() + 4 * self.pair_count()
    }

    /// 第 i 个头词。
    pub fn head(&self, i: usize) -> &'static str {
        // self.d 本身就是 'static（include_bytes!），拷出来就跟 &self 无关了
        let d = self.d;
        let base = self.head_off_pos() + 4 * i;
        let a = self.heads_pos() + self.u32_at(base) as usize;
        let b = self.heads_pos() + self.u32_at(base + 4) as usize;
        str::from_utf8(&d[a..b]).unwrap_or("")
    }

    /// 第 i 个头词的后续词迭代器。
    pub fn tails_at(&self, i: usize) -> Tails {
        let a = self.tail_off_pos() + 4 * i;
        Tails {
            d: self.d,
            tails_pos: self.tails_pos(),
            pool_pos: self.tail_pool_pos(),
            cur: self.u32_at(a) as usize,
            end: self.u32_at(a + 4) as usize,
        }
    }

    /// 精确查一个头词。二分在**排好序的头词池**上做，比较的是 UTF-8 字节。
    pub fn find(&self, key: &str) -> Option<Tails> {
        let (mut lo, mut hi) = (0usize, self.head_count());
        while lo < hi {
            let mid = (lo + hi) / 2;
            match self.head(mid).cmp(key) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(self.tails_at(mid)),
            }
        }
        None
    }

    /// 遍历所有头词（设置窗统计、单测用）。
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, Tails)> + '_ {
        (0..self.head_count()).map(move |i| (self.head(i), self.tails_at(i)))
    }
}

/// 一个头词的后续词迭代器。顺序就是生成时的词频顺序。
///
/// 不借用 [Bin]：数据切片本身是 'static（include_bytes!），把切片和段位置拷出来，
/// 迭代器就能独立于 Bin 存活，方法签名也不用玩生命周期杂技。
#[derive(Clone, Copy)]
pub struct Tails {
    d: &'static [u8],
    tails_pos: usize,
    pool_pos: usize,
    cur: usize,
    end: usize,
}

impl Iterator for Tails {
    type Item = &'static str;

    fn next(&mut self) -> Option<&'static str> {
        if self.cur >= self.end {
            return None;
        }
        let off = u32_at(self.d, self.tails_pos + 4 * self.cur) as usize;
        let p = self.pool_pos + off;
        let len = self.d[p] as usize;
        let w = str::from_utf8(&self.d[p + 1..p + 1 + len]).unwrap_or("");
        self.cur += 1;
        Some(w)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.end - self.cur;
        (n, Some(n))
    }
}

impl ExactSizeIterator for Tails {}

/// 把「头词 → 后续词（已按词频排好）」编码成二进制。**构建期**用（build.rs 直接
/// #[path] 把这个文件拉进去编译），单测也用它，保证编码器和解码器不会各改一半。
///
/// 内部会按头词字节序排序（二分的前提），重复头词只保留第一条。
pub fn encode(heads: &[(String, Vec<String>)]) -> Vec<u8> {
    let mut rows: Vec<&(String, Vec<String>)> = heads.iter().collect();
    rows.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    rows.dedup_by(|a, b| a.0 == b.0);

    let n = rows.len();
    let pairs: usize = rows.iter().map(|r| r.1.len()).sum();
    let mut head_pool = Vec::new();
    let mut tail_pool = Vec::new();
    let mut head_off = Vec::with_capacity(n + 1);
    let mut tail_off = Vec::with_capacity(n + 1);
    let mut tails = Vec::with_capacity(pairs);

    head_off.push(0u32);
    tail_off.push(0u32);
    for (head, words) in rows {
        head_pool.extend_from_slice(head.as_bytes());
        head_off.push(head_pool.len() as u32);
        for w in words {
            tails.push(tail_pool.len() as u32);
            let len = w.len().min(u8::MAX as usize);
            tail_pool.push(len as u8);
            tail_pool.extend_from_slice(&w.as_bytes()[..len]);
        }
        tail_off.push(tails.len() as u32);
    }

    let mut out =
        Vec::with_capacity(HEADER + 8 * (n + 1) + head_pool.len() + 4 * pairs + tail_pool.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(n as u32).to_le_bytes());
    out.extend_from_slice(&(pairs as u32).to_le_bytes());
    for v in &head_off {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for v in &tail_off {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&head_pool);
    for v in &tails {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&tail_pool);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaked(raw: Vec<u8>) -> &'static [u8] {
        Box::leak(raw.into_boxed_slice())
    }

    fn sample() -> &'static [u8] {
        leaked(encode(&[
            ("北京".to_string(), vec!["大学".to_string(), "时间".to_string()]),
            ("我".to_string(), vec!["觉得".to_string()]),
            ("计算机".to_string(), vec!["网络".to_string(), "软件".to_string()]),
        ]))
    }

    #[test]
    fn roundtrip_keeps_sorted_heads_and_order() {
        let bin = Bin::new(sample());
        assert_eq!(bin.head_count(), 3);
        assert_eq!(bin.pair_count(), 5);
        let heads: Vec<&str> = bin.iter().map(|(h, _)| h).collect();
        assert_eq!(heads, vec!["北京", "我", "计算机"], "按 UTF-8 字节序排");
        let beijing: Vec<&str> = bin.find("北京").unwrap().collect();
        assert_eq!(beijing, vec!["大学", "时间"], "顺序就是生成顺序");
        let me: Vec<&str> = bin.find("我").unwrap().collect();
        assert_eq!(me, vec!["觉得"]);
    }

    #[test]
    fn duplicates_keep_first_row() {
        let raw = encode(&[
            ("北京".to_string(), vec!["大学".to_string()]),
            ("北京".to_string(), vec!["时间".to_string()]),
        ]);
        let bin = Bin::new(leaked(raw));
        assert_eq!(bin.head_count(), 1);
        assert_eq!(bin.find("北京").unwrap().collect::<Vec<_>>(), vec!["大学"]);
    }

    #[test]
    fn broken_or_empty_data_is_an_empty_table() {
        for raw in [vec![], b"FHNB".to_vec(), vec![0u8; 64]] {
            let bin = Bin::new(leaked(raw));
            assert_eq!(bin.head_count(), 0);
            assert!(bin.find("北京").is_none());
            assert!(bin.iter().next().is_none());
        }
    }

    #[test]
    fn missing_head_returns_none() {
        let bin = Bin::new(sample());
        assert!(bin.find("上海").is_none());
        assert!(bin.find("").is_none());
    }
}
