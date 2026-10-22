//! 辅助码表：`字<TAB>自然码<TAB>小鹤形码`
//!
//! 数据由 `tools/gen_assist.py` 生成，来源 Unihan（部首）+ cjkvi-ids（部件分解）。
//!
//! - **自然码辅助码**：取部首的拼音首字母，1 键。例如 的→白→`b`、想→心→`x`
//! - **小鹤音形**：取首部件 + 末部件，各按声母定键，2 键。例如 打→扌(s)+丁(d)→`sd`
//!
//! 小鹤这一列是按官方「声母定键」规则 + IDS 分解算出来的，和官方码表在少数
//! 部件上有差异（官方末形有时取笔画，如 形 官方是 `kp`，这里是 `ks`）。
//! 作为候选筛选足够用，若要完全对齐官方，需要换成官方字根表。

use std::collections::HashMap;
use std::sync::OnceLock;

static RAW: &str = include_str!("assist.txt");

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

    pub fn label(&self) -> &'static str {
        match self {
            AssistMode::Off => "关闭",
            AssistMode::Ziranma => "自然码辅助码",
            AssistMode::Xiaohe => "小鹤音形",
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
        let c = code_of('打', AssistMode::Xiaohe).unwrap();
        assert_eq!(c.len(), 2, "打 should have 2-key code, got {c}");
        assert_eq!(code_of('打', AssistMode::Xiaohe), Some("sd"));
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
