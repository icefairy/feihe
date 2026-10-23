//! 二级简码（二简词）：两键打出一个常用两字词。
//!
//! 小鹤的「二简」是这么取的：**两个字各取自己双拼码的首键**，拼成两键。
//! `难道` = n(**n**an) + d(**d**ao) = `nd`；`所以` = s(**s**uo) + y(**y**i) = `sy`。
//! 于是常用的两字词也变成了「打两键 + 空格」就能出的东西——一简管单字，
//! 二简管词组，合起来是双拼用户提语速的主要手段。
//!
//! 数据在 `src/brief2.txt`（官方「二简词」表：首选 273 + 次选 97，共 370 条）。
//! 这里只管「查表 + 告诉调用方这个词该排第几」，排序仍然在 `dict::search` 里做。
//!
//! ## 首选 / 次选
//!
//! 两键码是有限资源，好的码早就被单字占住了：`mm` 是「面」，`uu` 是「书」，
//! `gg` 是「更」。官方把这类码上的词列成**次选**——它排在那个单字**后面**，
//! 不当第一候选。所以：
//!
//! - 首选（`word_for`）→ 提到第 0 位
//! - 次选（`alt_for`）→ 提到第 1 位
//!
//! 一个码同时有首选词和次选词时（`vd` 知道/直到、`vy` 这样/主意），
//! 正好占据前两位，第三位开始才是单字词频序。
//!
//! ## 和单字撞码是**故意**的
//!
//! 370 条里有 103 条的两键码本身是个合法音节（`nd`=奶、`sy`=孙、`lj`=蓝）。
//! 这不是抄错——小鹤就是这么设计的：二简词优先，想打那个字再按数字选，
//! 或者用置顶把它钉回去。真嫌碍事，把那一行从 `brief2.txt` 里删掉即可（不用改代码）。
//!
//! ## 为什么不分方案
//!
//! 表里的码是按**小鹤**键位算的。微软双拼的键位不同（零声母用 `o` 占位，
//! `mm` 是 `mian`、`nd` 是 `nai` 的对应关系全变了），拿小鹤的表去套只会张冠李戴。
//! 所以调用方只在 `Scheme::Xiaohe` 下查，见 `dict::search`。

use std::sync::OnceLock;

/// 表源文件。第一次查询时解析一次，之后只读。
const RAW: &str = include_str!("brief2.txt");

/// 一条二简词：码 / 词 / 是否次选。
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub code: &'static str,
    pub word: &'static str,
    pub alt: bool,
}

static TABLE: OnceLock<Vec<Entry>> = OnceLock::new();

fn table() -> &'static [Entry] {
    TABLE.get_or_init(|| RAW.lines().filter_map(parse_line).collect())
}

fn parse_line(line: &'static str) -> Option<Entry> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let mut it = line.split_whitespace();
    let code = it.next()?;
    let word = it.next()?;
    Some(Entry {
        code,
        word,
        alt: it.next() == Some("次"),
    })
}

/// 恰好两键的编码才可能是二简词码。
fn is_brief_code(code: &str) -> bool {
    code.len() == 2 && code.bytes().all(|b| b.is_ascii_lowercase())
}

fn find(code: &str, alt: bool) -> Option<&'static str> {
    if !is_brief_code(code) {
        return None;
    }
    let t = table();
    // 表按码升序，同码的几条连续（首选在前、次选在后）
    let start = t.partition_point(|e| e.code < code);
    t[start..]
        .iter()
        .take_while(|e| e.code == code)
        .find(|e| e.alt == alt)
        .map(|e| e.word)
}

/// 这个两键码对应的**首选**二简词。
pub fn word_for(code: &str) -> Option<&'static str> {
    find(code, false)
}

/// 这个两键码对应的**次选**二简词（排在该码的单字后面）。
pub fn alt_for(code: &str) -> Option<&'static str> {
    find(code, true)
}

/// 表里一共多少条。只给单测用，发布构建里没有调用方。
#[cfg(test)]
pub fn count() -> usize {
    table().len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_sorted_and_well_formed() {
        assert_eq!(count(), 370, "官方表 273 首选 + 97 次选");
        let t = table();
        let mut bad = Vec::new();
        for (i, e) in t.iter().enumerate() {
            if !is_brief_code(e.code) {
                bad.push(format!("第 {i} 条码不是两键小写：{} {}", e.code, e.word));
            }
            if e.word.chars().count() != 2 {
                bad.push(format!("{} 不是两字词", e.word));
            }
            if i > 0 && t[i - 1].code > e.code {
                bad.push(format!("{} 排在 {} 后面，不是升序", e.code, t[i - 1].code));
            }
        }
        assert!(bad.is_empty(), "brief2.txt 有问题：\n{}", bad.join("\n"));
    }

    #[test]
    fn no_duplicate_code_and_tier() {
        let t = table();
        for w in t.windows(2) {
            assert!(
                !(w[0].code == w[1].code && w[0].alt == w[1].alt),
                "{} 有两条第 {} 位的词：{} / {}",
                w[0].code,
                if w[0].alt { "次选" } else { "首选" },
                w[0].word,
                w[1].word
            );
        }
    }

    #[test]
    fn no_duplicate_word() {
        let mut seen = std::collections::HashSet::new();
        let mut dup = Vec::new();
        for e in table() {
            if !seen.insert(e.word) {
                dup.push(e.word);
            }
        }
        assert!(dup.is_empty(), "同一个词出现多次：{dup:?}");
    }

    #[test]
    fn primary_hits() {
        assert_eq!(word_for("nd"), Some("难道"));
        assert_eq!(word_for("sy"), Some("所以"));
        assert_eq!(word_for("wm"), Some("我们"));
        assert_eq!(word_for("py"), Some("拼音"));
    }

    #[test]
    fn alt_hits() {
        // mm 的码本来归「面」，所以「慢慢」是次选
        assert_eq!(alt_for("mm"), Some("慢慢"));
        assert_eq!(word_for("mm"), None);
        assert_eq!(alt_for("uu"), Some("试试"));
    }

    #[test]
    fn same_code_can_have_both_tiers() {
        assert_eq!(word_for("vd"), Some("知道"));
        assert_eq!(alt_for("vd"), Some("直到"));
        assert_eq!(word_for("vy"), Some("这样"));
        assert_eq!(alt_for("vy"), Some("主意"));
    }

    #[test]
    fn only_two_letter_codes() {
        for code in ["", "n", "ndx", "ni'hao", "ND", "n1"] {
            assert_eq!(word_for(code), None, "{code} 不该有二简词");
            assert_eq!(alt_for(code), None, "{code} 不该有二简词");
        }
    }
}
