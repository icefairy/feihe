//! 反查：把一段汉字拆成「每个字的音 + 辅助码」，给 `/fc` 快捷命令用。
//!
//! ## 干什么
//!
//! 用户从别处复制一个生僻词（「璇玑」「饕餮」），想知道**本输入法里该敲什么**。
//! `/fc` 读剪贴板，输出两行候选：
//!
//! - `璇 xuan+wr  玑 ji+wj` —— 逐字明细（音 = 全拼，形 = 当前辅助码方案）
//! - `璇玑  xrji → wr`      —— 可以直接照着敲的连打串（`→` 后面是 Tab 之后的辅助码）
//!
//! ## 为什么音要按全拼显示、连打却按双拼码
//!
//! 两个问题的答案不一样：「这个字念什么」的答案是全拼（`xuan`），「这个字怎么打」
//! 的答案是当前方案的码（小鹤 `xr`）。硬要合成一处，必然有一半是错的。
//!
//! ## 读音从哪来
//!
//! 词库里每个单字条目都带着它的全拼（`dict::readings_for`），多音字会返回多条。
//! 这个词库就是打字时用的那一个，所以反查出来的码**一定能敲出来**——用外部字典
//! 反查会和词库对不上，用户照着敲反而打不出字。

use crate::assist::{self, AssistMode};
use crate::config;
use crate::dict;
use crate::shuangpin::{self, Scheme};

/// 一次最多反查几个字。再长就不是「一个词」了，输出会长到候选窗放不下。
const MAX_CHARS: usize = 8;

/// 一个多音字最多列几个读音。
const MAX_READINGS: usize = 4;

fn is_han(c: char) -> bool {
    matches!(
        c as u32,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F
    )
}

/// 单个音节的「音」部分：全拼模式直接去掉空格，双拼模式编码成当前方案的码。
///
/// 编码失败（词库里塞了非法音节）时退回全拼串——显示一个敲不出来的码，
/// 不如显示全拼让用户自己判断。
fn sound_code(pinyin: &str, scheme: Scheme, flat: bool) -> String {
    let plain: String = pinyin.chars().filter(|c| !c.is_whitespace()).collect();
    if flat {
        return plain;
    }
    shuangpin::encode_pinyin(scheme, pinyin).unwrap_or(plain)
}

/// 反查一段文本，返回候选（第一条是逐字明细，第二条是连打串）。
///
/// 抽成纯函数（剪贴板读取在 `from_clipboard`）才能单测：`Scheme` / `AssistMode`
/// 都从参数进来，不依赖进程级全局状态。
pub fn describe(text: &str, scheme: Scheme, mode: AssistMode, flat: bool) -> Vec<String> {
    let mut chars: Vec<char> = text.chars().filter(|c| is_han(*c)).collect();
    if chars.is_empty() {
        return Vec::new();
    }
    let truncated = chars.len() > MAX_CHARS;
    chars.truncate(MAX_CHARS);

    let readings = dict::readings_for(&chars);
    let mut detail: Vec<String> = Vec::with_capacity(chars.len());
    // 连打串：每个字取第一个读音
    let mut song = String::new();
    let mut first_assist: Option<&'static str> = None;

    for (i, &ch) in chars.iter().enumerate() {
        let assist_code = assist::code_of(ch, mode);
        if i == 0 {
            first_assist = assist_code;
        }
        let rs = &readings[i];
        if rs.is_empty() {
            // 词库里没有这个字的单字条目：音未知，形码照样给（辅助码表比词库大）
            detail.push(match assist_code {
                Some(a) => format!("{ch} ?+{a}"),
                None => format!("{ch} ?"),
            });
            continue;
        }
        let mut shown: Vec<String> = Vec::with_capacity(rs.len());
        for (k, py) in rs.iter().enumerate().take(MAX_READINGS) {
            let snd = sound_code(py, scheme, flat);
            if k == 0 {
                song.push_str(&snd);
            }
            shown.push(match assist_code {
                Some(a) => format!("{snd}+{a}"),
                None => snd,
            });
        }
        detail.push(format!("{ch} {}", shown.join(" / ")));
    }

    if detail.is_empty() {
        return Vec::new();
    }
    let mut first = detail.join("  ");
    if truncated {
        first.push_str(" …");
    }

    // 连打串：单字就是「音码 + 形码」连写（正好等于反查工具给出的码）；
    // 多字是把各字的音码连起来，辅助码只能 Tab 之后加**首字**的（见
    // `assist::code_of_word`：词只按首字筛选）。
    if song.is_empty() {
        return vec![first];
    }
    let second = match first_assist {
        Some(a) if chars.len() == 1 => format!("{song}{a}"),
        Some(a) => format!("{song} → {a}"),
        None => song,
    };
    vec![first, second]
}

/// `/fc` 的候选：读剪贴板 → 反查。
pub fn from_clipboard() -> Vec<String> {
    let Some(text) = crate::clipboard::text() else {
        return vec!["（读不到剪贴板，稍等一下再试）".to_string()];
    };
    let cfg = config::get();
    let out = describe(&text, cfg.scheme, cfg.assist_mode, cfg.pinyin_mode);
    if out.is_empty() {
        vec!["（剪贴板里没有汉字）".to_string()]
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_char_gives_the_reverse_lookup_code() {
        // 官方反查：玑 = ji + wj，连打就是 jiwj
        let r = describe("玑", Scheme::Xiaohe, AssistMode::Xiaohe, false);
        println!("{r:?}");
        assert!(r[0].starts_with("玑 ji+wj"), "{r:?}");
        assert_eq!(r[1], "jiwj", "{r:?}");
    }

    #[test]
    fn word_lists_every_char_and_the_first_assist() {
        let r = describe("璇玑", Scheme::Xiaohe, AssistMode::Xiaohe, false);
        println!("{r:?}");
        // 逐字明细里两个字都在，且各自带形码
        assert!(r[0].contains("璇 xr+wr"), "{r:?}");
        assert!(r[0].contains("玑 ji+wj"), "{r:?}");
        // 连打串 = 两字双拼码 + 首字形码
        assert_eq!(r[1], "xrji → wr", "{r:?}");
    }

    #[test]
    fn full_pinyin_mode_spells_it_out() {
        let r = describe("璇玑", Scheme::Xiaohe, AssistMode::Xiaohe, true);
        println!("{r:?}");
        assert!(r[0].contains("璇 xuan+wr"), "{r:?}");
        assert_eq!(r[1], "xuanji → wr", "{r:?}");
    }

    #[test]
    fn ziranma_assist_is_one_key() {
        let r = describe("的", Scheme::Xiaohe, AssistMode::Ziranma, false);
        println!("{r:?}");
        assert!(r[0].contains("的 de+b"), "{r:?}");
        assert_eq!(r[1], "deb");
    }

    /// 辅助码关掉时不该冒出形码，也不该留一个空的 `+`
    #[test]
    fn assist_off_shows_sound_only() {
        let r = describe("玑", Scheme::Xiaohe, AssistMode::Off, false);
        println!("{r:?}");
        assert_eq!(r[0], "玑 ji");
        assert_eq!(r[1], "ji");
    }

    #[test]
    fn non_chinese_is_ignored() {
        assert!(describe("hello, world", Scheme::Xiaohe, AssistMode::Xiaohe, false).is_empty());
        assert!(describe("", Scheme::Xiaohe, AssistMode::Xiaohe, false).is_empty());
        // 混排时只反查汉字
        let r = describe("a璇1", Scheme::Xiaohe, AssistMode::Xiaohe, false);
        assert_eq!(r[0], "璇 xr+wr", "{r:?}");
    }

    #[test]
    fn long_text_is_capped() {
        let r = describe("中华人民共和国万岁万岁万万岁", Scheme::Xiaohe, AssistMode::Xiaohe, false);
        assert!(r[0].ends_with('…'), "{r:?}");
        // 8 个字 + 7 个分隔 + 截断符，长度有限
        assert!(r[0].chars().count() < 100, "{r:?}");
    }
}
