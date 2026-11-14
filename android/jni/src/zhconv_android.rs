//! Android 端的简体 → 繁体转换器（core 的 `zhconv::Converter` 实现）。
//!
//! ## 为什么要有这个文件
//!
//! core 的 `zhconv::set_converter` 是个「壳注入」接口：Windows 壳用系统 NLS
//! （`LCMapStringEx`）注册，而 Android 没有等价的系统 API。`initNative` 里
//! 以前**漏了这一步注册**（docs/Android-方案.md 早就标「必须换」但一直没闭环），
//! 于是「输出繁体」开关只写进了 config.ini，`CONVERTER` 永远为空，`apply()`
//! 走 None 兜底原样返回简体——用户 2026-11-14 报的「勾了繁体，打字还是简体」。
//!
//! 这里放一张自带字表（[`s2t.txt`]，OpenCC s2t 词典逐字转换生成），零外部依赖、
//! 零构建步骤，和 Windows 用系统表是同一种「壳提供映射」的思路。
//!
//! ## 粒度
//!
//! 逐字映射，不做词组消歧（同 core/src/zhconv.rs 的约定，和搜狗/微软拼音一致）。
//! 一简对多繁的字（发→發/髮）取 s2t 主映射，这是逐字方案的固有取舍。

use std::collections::HashMap;
use std::sync::OnceLock;

const RAW: &str = include_str!("s2t.txt");

static MAP: OnceLock<HashMap<char, char>> = OnceLock::new();

/// 一个字符串恰好是一个字符时返回它，否则 None（过滤多字/空串脏数据）。
fn single_char(s: &str) -> Option<char> {
    let mut it = s.chars();
    match (it.next(), it.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    }
}

fn table() -> &'static HashMap<char, char> {
    MAP.get_or_init(|| {
        let mut m = HashMap::with_capacity(3000);
        for line in RAW.lines() {
            let line = line.trim();
            // 跳过注释/空行/格式不对的行
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut it = line.split('\t');
            let (Some(s0), Some(t0)) = (it.next(), it.next()) else { continue };
            // 只认「单字→单字」：两边各自恰好一个 char，且没有第三列脏数据
            let (Some(s), Some(t)) = (single_char(s0), single_char(t0)) else { continue };
            m.insert(s, t);
        }
        m
    })
}

/// 签名严格匹配 `feihe_core::zhconv::Converter = fn(&str) -> Option<String>`。
///
/// - 纯 ASCII 直接返回原串（core 已有快路径，这里再兜一层，便于单测独立跑）；
/// - 繁体字/表里没有的字保持原样 → 幂等（繁体再转不变）；
/// - 永返回 `Some`，因为「转换失败丢字」比「留着简体」严重得多，而这张表
///   是自含的、不存在运行时失败。
pub fn to_traditional(s: &str) -> Option<String> {
    if s.is_empty() || s.is_ascii() {
        return Some(s.to_string());
    }
    let m = table();
    Some(s.chars().map(|c| m.get(&c).copied().unwrap_or(c)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_common_words() {
        assert_eq!(to_traditional("设置").as_deref(), Some("設置"));
        assert_eq!(to_traditional("简体中文输入法").as_deref(), Some("簡體中文輸入法"));
    }

    #[test]
    fn ascii_is_untouched() {
        assert_eq!(to_traditional("vhsj,。123").as_deref(), Some("vhsj,。123"));
    }

    #[test]
    fn keeps_length() {
        for s in ["第1个设置", "头发", "𠀀设置"] {
            let out = to_traditional(s).unwrap();
            assert_eq!(out.chars().count(), s.chars().count(), "{s} 字数变了");
        }
    }

    #[test]
    fn is_idempotent() {
        let once = to_traditional("指示器按设繁体").unwrap();
        let twice = to_traditional(&once).unwrap();
        assert_eq!(once, twice, "繁体再转一次不该继续变");
    }

    #[test]
    fn table_is_loaded() {
        // 表规模是「数据没被 include 歪」的哨兵
        let n = table().len();
        assert!(n > 2000, "s2t 表只有 {n} 条，明显没读全");
        assert_eq!(table().get(&'设').copied(), Some('設'));
    }
}
