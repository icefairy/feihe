//! 拼写纠错：码里有一个音节「根本不存在」时，试着一键修回来。
//!
//! ## 和全拼的纠错不是一回事
//!
//! 全拼的纠错是编辑距离：多打一个字母、少打一个、敲反两个。**双拼不行**——每个音节
//! 恰好两键，删一个或加一个会让后面所有音节全部错位，救回来的概率比不救还低。
//!
//! 双拼能救的形态只有两种：
//!
//! 1. **换一个键**（按错了）：`nihx` 的第二个音节 `h` + `x`(=ia) 是 `hia`，普通话里
//!    没有这个音节；把 `x` 换成 `c`(=ao) 就是 `nihc` = 你好。
//! 2. 相邻两键敲反 —— 但双拼里 `hc` 和 `ch` 是两个完全不同的音节，换位之后
//!    **两个音节都变**，不是「一个音节的拼写错了」。所以这里不做换位：
//!    换位属于「整串打乱了」，靠用户自己退格重打更快。
//!
//! ## 只在「有非法音节」时纠错，不在「查不到词」时纠错
//!
//! 这是本模块最关键的一条判断。两种情况看着都像打错了，但性质完全不同：
//!
//! - **有非法音节** = 这个两键组合对应不了任何普通话音节。`hx`(`hia`) 不存在，
//!   说明这个键按错了——**确定性信号**，修它不会冤枉人。
//! - **全合法但查不到词** = 码本身是正经双拼码，只是这几个音节凑在一起没有词。
//!   这时「换一键」的变体几乎**全都有词**（合法音节组合大多有词），给出来的一屏
//!   候选里可能一个都不是用户想要的，还会把「我打的是生僻词」变成「怎么冒出一堆
//!   不相干的词」。宁可不纠。
//!
//! ## 只修「恰好一个」非法音节
//!
//! 两个音节同时非法 = 至少按错两个键（或者整串错位），这种情况下改动量至少 2 键，
//! 变体数按平方涨，而正确率断崖下跌。一个键按错才是日常。
//!
//! ## 怎么判断「哪个键按错了」
//!
//! 一开始想用「哪个音节在词库里查不到字」。**这条路在双拼里走不通**——小鹤的
//! 26 个韵母键把字母表铺满了，`h` + `x`(`ua`) 是 `hua`（话），`g` + `p`(`ie`) 是
//! `gie`……等等，`hx` 其实有字。结构上不存在的组合很少（只有 `a`/`e`/`o` 开头的
//! 零声母有约束），所以「查不到字的音节」几乎不存在。
//!
//! 真正的情况是：**每个音节单独都有字，凑在一起没有词**。`nihx` 是两个正经音节
//! `ni` + `hua`，只是「你话」不在词库里——用户想打的是「你好」，`c` 按成了 `x`。
//!
//! 所以 `variants` 不猜「哪个音节坏了」，而是**每个音节位置都试一遍**，把
//! `nihc`（你好）、`niha`（你哈）…… 全列出来，由调用方查词库决定哪些能出词。
//!
//! ## 顺序：最后按的键最先试
//!
//! 打错的是**刚按下去的那个键**，所以从最后一个音节往前试，每个音节内部先换第二键
//! （韵母位）再换第一键。调用方按这个顺序取候选，用户看到的第一个就是「最可能想
//! 打的那个」——`nihx` 的第一条候选是 `nihc`（你好），不是从前一个音节瞎改出来的。
//!
//! ## 返回的是「码」，不是「词」
//!
//! 查词库、按词频排、和别的候选合并都在 `dict::search` 里做——那里才有词库和
//! 词频。这里只负责「哪些码值得再查一遍」，纯函数，能直接单测。

use crate::shuangpin::{is_valid_pair, Scheme};

/// 一次「换一个键」的修复。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fix {
    /// 改的是第几个字符（全是 ASCII，字符下标 = 字节下标）
    pub at: usize,
    /// 原来那个键
    pub was: char,
    /// 换成哪个键
    pub now: char,
    /// 修好之后的码
    pub code: String,
}

/// 最短多少个键才纠错：4 键 = 两个音节。
///
/// 单音节（2 键）不纠：一共就两个键，光靠「换一个键」就能拼出几十个合法音节，
/// 给出来的一屏候选全是噪音，用户还不如自己重打。
const MIN_LEN: usize = 4;

/// 最长多少个键：12 个音节。再长多半是在打长句，不是按错了一个键。
const MAX_LEN: usize = 24;

/// 每个音节位置最多生成多少个变体。一个位置理论上最多 50 个（两个键各 25 个），
/// 截到 32 是为了让后面的音节位置也轮得到——不然变体全被最后一个音节占满，
/// 「第一个音节打错」这种照样救不回来。
const MAX_PER_SYLLABLE: usize = 32;

/// 一次最多返回多少个变体。调用方按顺序查库、取前几条有词的，这个上限只是
/// 保证一次按键最多多查这么多次词库（96 次二分，微秒级）。
const MAX_VARIANTS: usize = 96;

/// 这段码值不值得试纠错：偶数长度（完整音节）、长度合适、纯小写字母。
///
/// **偶数**这条不能少：奇数长度说明用户还在打（`nih` 是 `ni` + 半个 `hao`），
/// 这时候纠错会让候选窗在半截码上突然冒出一堆词。
pub fn usable(code: &str) -> bool {
    let n = code.len();
    (MIN_LEN..=MAX_LEN).contains(&n)
        && n.is_multiple_of(2)
        && code.bytes().all(|b| b.is_ascii_lowercase())
}

/// 生成「把某一个音节换掉一个键」的候选码，按「最可能是用户想打的」排序。
///
/// 排序规则：最后一个音节在前（刚按的键最容易错），同一音节内先换第二键（韵母位）
/// 再换第一键。调用方顺着取，前几条就是最可能的。
///
/// 只有一件事这里判不了：**换出来的码有没有词**。那要查词库，交给调用方——
/// 本模块因此仍是纯逻辑，测试不依赖真实词库内容。
pub fn variants(code: &str, scheme: Scheme) -> Vec<Fix> {
    if !usable(code) {
        return Vec::new();
    }
    let bytes = code.as_bytes();
    let syllables = bytes.len() / 2;
    let mut out = Vec::with_capacity(MAX_VARIANTS);
    // 从最后一个音节往前
    for i in (0..syllables).rev() {
        let (a, b) = (bytes[i * 2] as char, bytes[i * 2 + 1] as char);
        let before = out.len();
        // 先换第二键（韵母位），再换第一键（声母位）
        for pos in [1usize, 0] {
            let orig = if pos == 0 { a } else { b };
            let keep = if pos == 0 { b } else { a };
            for c in b'a'..=b'z' {
                let c = c as char;
                if c == orig {
                    continue;
                }
                // 结构上不可能的先挡掉（便宜），剩下的由调用方查词库
                let ok = if pos == 0 {
                    is_valid_pair(scheme, c, keep)
                } else {
                    is_valid_pair(scheme, keep, c)
                };
                if !ok {
                    continue;
                }
                out.push(replace(code, i * 2 + pos, orig, c));
                if out.len() - before >= MAX_PER_SYLLABLE {
                    break;
                }
            }
            if out.len() - before >= MAX_PER_SYLLABLE {
                break;
            }
        }
        if out.len() >= MAX_VARIANTS {
            break;
        }
    }
    out.truncate(MAX_VARIANTS);
    out
}

fn replace(code: &str, at: usize, was: char, now: char) -> Fix {
    let mut s = String::with_capacity(code.len());
    s.push_str(&code[..at]);
    s.push(now);
    s.push_str(&code[at + 1..]);
    Fix {
        at,
        was,
        now,
        code: s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(v: &[Fix]) -> Vec<&str> {
        v.iter().map(|f| f.code.as_str()).collect()
    }

    #[test]
    fn usable_needs_complete_syllables() {
        assert!(usable("nihc"));
        assert!(usable("vsgo"));
        // 奇数长度 = 还在打，不能纠
        assert!(!usable("nih"));
        assert!(!usable("n"));
        // 太短（一个音节）
        assert!(!usable("ni"));
        // 太长
        assert!(!usable(&"ni".repeat(13)));
        // 非小写字母
        assert!(!usable("ni'hc"));
        assert!(!usable("/rq"));
    }

    /// 最后一个音节先试，且音节内先换第二键——那是用户刚按下去的那个键。
    #[test]
    fn last_syllable_and_its_second_key_come_first() {
        let v = variants("nihx", Scheme::Xiaohe);
        assert!(!v.is_empty());
        assert_eq!(v[0].at, 3, "第一条该是换第二音节的第二键：{v:?}");
        assert_eq!(v[0].was, 'x');

        let at = |f: &Fix| f.at;
        // 所有「换最后一个音节」的变体都排在「换第一个音节」的前面
        let last_of_last = v.iter().rposition(|f| at(f) >= 2).expect("最后音节得有变体");
        let first_of_first = v.iter().position(|f| at(f) < 2).expect("第一个音节也得有变体");
        assert!(
            last_of_last < first_of_first,
            "最后一个音节的变体该整批排在前面"
        );
        // 同一音节内，先换第二键（4 个下标里靠后的那个）再换第一键
        let last_second = v.iter().rposition(|f| at(f) == 3).expect("得换第二键");
        let first_first = v.iter().position(|f| at(f) == 2).expect("也得换第一键");
        assert!(last_second < first_first, "同音节内先换第二键");
    }

    /// 主力场景：`nihx`（你话）要能列出 `nihc`（你好）。
    #[test]
    fn lists_the_word_the_user_meant() {
        let v = variants("nihx", Scheme::Xiaohe);
        let all = codes(&v);
        assert!(all.contains(&"nihc"), "{all:?}");
        // 每个变体都只动一个键
        for f in &v {
            assert_eq!(f.code.len(), 4);
            assert_eq!(&f.code[..f.at], &"nihx"[..f.at]);
            assert_eq!(&f.code[f.at + 1..], &"nihx"[f.at + 1..]);
        }
        // 原码不在里面
        assert!(!all.contains(&"nihx"));
        // 没有重复
        let mut sorted = all.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), all.len(), "变体有重复：{all:?}");
    }

    /// 三个音节也能纠，且每个音节位置都轮得到。
    #[test]
    fn every_syllable_position_gets_a_turn() {
        let v = variants("nihcma", Scheme::Xiaohe);
        for pos in [0usize, 1, 2] {
            assert!(
                v.iter().any(|f| f.at == pos * 2 || f.at == pos * 2 + 1),
                "第 {pos} 个音节一个变体都没有：{v:?}"
            );
        }
    }

    /// 变体里每一个都必须结构合法——换出来的音节不能是双拼根本拼不出的东西。
    #[test]
    fn variants_are_structurally_valid() {
        for f in variants("nihx", Scheme::Xiaohe) {
            let bs = f.code.as_bytes();
            for i in 0..2 {
                assert!(
                    is_valid_pair(Scheme::Xiaohe, bs[i * 2] as char, bs[i * 2 + 1] as char),
                    "{} 的第 {} 个音节结构非法",
                    f.code,
                    i
                );
            }
        }
    }

    /// 数量有上限：一次按键不该多查上百次词库。
    #[test]
    fn variant_count_is_bounded() {
        for n in [2, 3, 6, 12] {
            let code = "ni".repeat(n);
            let v = variants(&code, Scheme::Xiaohe);
            assert!(v.len() <= MAX_VARIANTS, "{n} 音节给了 {} 条", v.len());
            // 每个位置至少有机会出现
            assert!(v.iter().any(|f| f.at / 2 == n - 1), "最后一个音节没轮到");
        }
    }

    /// 微软双拼的合法音节表不一样，换出来的东西也要按它自己的表判。
    #[test]
    fn schemes_have_their_own_tables() {
        let mut differs = Vec::new();
        for a in b'a'..=b'z' {
            for b in b'a'..=b'z' {
                let (a, b) = (a as char, b as char);
                if is_valid_pair(Scheme::Xiaohe, a, b) != is_valid_pair(Scheme::Ms, a, b) {
                    differs.push((a, b));
                }
            }
        }
        assert!(!differs.is_empty(), "两种方案的音节表不可能完全一样");
        assert!(differs.len() > 10, "差异太少了，表可能贴错了：{differs:?}");

        // 同一段码在两种方案下给出的变体也不同（各自的合法音节不同）
        let xh = variants("nihx", Scheme::Xiaohe);
        let ms = variants("nihx", Scheme::Ms);
        assert_ne!(codes(&xh), codes(&ms));
    }

    #[test]
    fn fix_records_what_changed() {
        let f = replace("niha", 3, 'a', 'c');
        assert_eq!(
            f,
            Fix {
                at: 3,
                was: 'a',
                now: 'c',
                code: "nihc".into()
            }
        );
    }
}
