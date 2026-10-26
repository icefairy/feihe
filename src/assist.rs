//! 辅助码表：`字<TAB>自然码<TAB>小鹤形码`
//!
//! 数据由 `tools/gen_assist.py` 生成，来源 Unihan（部首）+ 小鹤官方字码：
//!
//! - **自然码辅助码**：取部首的拼音首字母，1 键。例如 的→白→`b`、想→心→`x`
//! - **小鹤音形**：取首部件 + 末部件，各按声母定键，2 键。例如 带→`fj`（带上、巾）、
//!   打→`fd`（扌、丁）。**这一列必须是官方码**，见下。
//!
//! 小鹤这一列曾经是「按 IDS 部件分解 + 声母定键」推算的。实测和官方码只有 **12%**
//! 一致（1772 字对比）：官方有 160 多个专用字根（带上、寿上、具上、既左、余下…）、
//! 有「孰大优先 / 相交不拆 / 插隔不取」这些需要笔顺几何才能判的规则，光靠 IDS
//! 树推不出来。用户按官方反查工具的码敲辅助码却一个字都筛不出来，比没有辅助码更糟。
//! 所以现在只认官方数据（`tools/fetch_xhup.py` 一次性抓、结果进 `tools/xhup_cache.txt`），
//! 推算那条路整个删掉了——**错的码不会报错，只会静默筛空**。

use std::collections::HashMap;
use std::sync::OnceLock;

/// 原始表文本。`pub(crate)` 只给单测做格式校验用（格式错了筛选会静默失效）。
pub(crate) static RAW: &str = include_str!("assist.txt");

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AssistMode {
    Off,
    Ziranma,
    Xiaohe,
}

impl AssistMode {
    pub fn from_key(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "ziranma" | "zrm" | "自然码" => AssistMode::Ziranma,
            "xiaohe" | "xh" | "小鹤" => AssistMode::Xiaohe,
            _ => AssistMode::Off,
        }
    }

    pub fn key(&self) -> &'static str {
        match self {
            AssistMode::Off => "off",
            AssistMode::Ziranma => "ziranma",
            AssistMode::Xiaohe => "xiaohe",
        }
    }
}

type Table = HashMap<char, (&'static str, &'static str)>;

fn table() -> &'static Table {
    static T: OnceLock<Table> = OnceLock::new();
    T.get_or_init(|| {
        let mut map = HashMap::with_capacity(4096);
        for line in RAW.lines() {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            let mut it = line.split('\t');
            let (Some(ch), Some(zrm), Some(xh)) = (it.next(), it.next(), it.next()) else {
                continue;
            };
            let Some(ch) = ch.chars().next() else { continue };
            map.insert(ch, (zrm, xh));
        }
        map
    })
}

/// 取某个字的辅助码。`mode = Off` 时永远返回 None。
pub fn code_of(ch: char, mode: AssistMode) -> Option<&'static str> {
    let (zrm, xh) = *table().get(&ch)?;
    let s = match mode {
        AssistMode::Off => return None,
        AssistMode::Ziranma => zrm,
        AssistMode::Xiaohe => xh,
    };
    if s == "-" || s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// 整个词取首字的辅助码（音形方案里词组就按首字筛选）
pub fn code_of_word(word: &str, mode: AssistMode) -> Option<&'static str> {
    if mode == AssistMode::Off {
        return None;
    }
    code_of(word.chars().next()?, mode)
}

/// 一个字的码是否命中输入的前缀。
///
/// 小鹤那一列可能是多位（`fj|gs`）：多音字在不同读音下官方给的形码不一样，
/// 也可能一个读音有官方码 + 兼容码。**任一位命中就算命中**——按第一段算会让
/// 另一半读音的字永远筛不出来，用户看到的就是「辅助码对我这个字无效」。
pub fn matches(code: &str, prefix: &str) -> bool {
    if prefix.is_empty() {
        return true;
    }
    code.split('|').any(|one| one.starts_with(prefix))
}

/// 候选词是否命中辅助码前缀（只按首字算，和 `code_of_word` 一致）。
pub fn word_matches(word: &str, mode: AssistMode, prefix: &str) -> bool {
    if prefix.is_empty() {
        return true;
    }
    code_of_word(word, mode).is_some_and(|c| matches(c, prefix))
}

/// 表里有多少字（自检用）
#[allow(dead_code)]
pub fn len() -> usize {
    table().len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_loaded() {
        assert!(len() > 3000, "assist table too small: {}", len());
    }

    #[test]
    fn ziranma_is_radical_initial() {
        // 的 -> 白 -> b；想 -> 心 -> x；你 -> 人 -> r
        assert_eq!(code_of('的', AssistMode::Ziranma), Some("b"));
        assert_eq!(code_of('想', AssistMode::Ziranma), Some("x"));
        assert_eq!(code_of('你', AssistMode::Ziranma), Some("r"));
    }

    #[test]
    fn xiaohe_is_two_keys() {
        // 官方码：扌(f) + 丁(d)。这条被 react.xhup.club 反查钉死，别再改回去
        let c = code_of('打', AssistMode::Xiaohe).unwrap();
        assert_eq!(c, "fd");
        assert!(matches(c, "f"));
        assert!(matches(c, "fd"));
        assert!(!matches(c, "s"), "打 不是 s 开头");
    }

    #[test]
    fn multi_code_matches_any() {
        // 多音字官方可能给两个形码：任一位命中都算命中
        assert!(matches("fj|gs", "gs"));
        assert!(matches("fj|gs", "f"));
        assert!(!matches("fj|gs", "z"));
        assert!(matches("", ""), "空前缀永远命中");
    }

    #[test]
    fn word_matches_uses_first_char() {
        assert!(word_matches("带上", AssistMode::Xiaohe, "fj"));
        assert!(!word_matches("带上", AssistMode::Xiaohe, "zz"));
        assert!(word_matches("带上", AssistMode::Xiaohe, ""));
    }

    #[test]
    fn off_mode_returns_none() {
        assert_eq!(code_of('的', AssistMode::Off), None);
    }

    #[test]
    fn word_uses_first_char() {
        assert_eq!(
            code_of_word("你好", AssistMode::Ziranma),
            code_of('你', AssistMode::Ziranma)
        );
    }
}
