//! 简繁转换：中文模式下「繁体」开关的实现。
//!
//! ## 为什么不自带字表
//!
//! 常见做法是塞一张 `s2t.txt`（三四千对，UTF-8 约 30 KB）再二分查表。这里改用
//! Windows 自带的 NLS 映射表——`LCMapStringEx` + `LCMAP_TRADITIONAL_CHINESE`。
//! 好处：零体积、零构建步骤、系统升级时字表跟着更新；代价是依赖系统区域数据
//! （简体中文 Windows 一定装，其它语言的 Windows 也带着 zh-CN 的 NLS 数据）。
//!
//! 拿不到就原样返回，**绝不因为转换失败丢字**。
//!
//! ## 转换粒度
//!
//! 逐字映射，不做词组消歧（「头发」→「頭髮」而不是「頭发」是靠 NLS 表里
//! 那个字本身的多对一关系，不是上下文判断）。搜狗/微软拼音也是这个粒度，
//! 对输入法足够——真正需要词组级消歧的场合用户会自己改。

use std::borrow::Cow;

use windows::core::PCWSTR;
use windows::Win32::Globalization::{LCMapStringEx, LCMAP_TRADITIONAL_CHINESE};

/// 优先用的 locale。文档要求 `LCMAP_TRADITIONAL_CHINESE` 必须配中文 locale，
/// 用 `LOCALE_NAME_SYSTEM_DEFAULT` 在英文系统上会失败，所以显式指定。
const LOCALE: PCWSTR = windows::core::w!("zh-CN");

/// 简体 → 繁体。
///
/// 纯 ASCII（编码、标点、数字）直接原样返回，不做任何系统调用——这条快路径
/// 覆盖了绝大多数调用，因为组合串和候选里的短词经常是纯字母/纯标点。
///
/// 对已经是繁体的文本再转一次是幂等的（繁体字映射到自身），所以「候选窗转一遍、
/// 上屏时再转一遍」不会出问题。
pub fn to_traditional(s: &str) -> Cow<'_, str> {
    if s.is_empty() || s.is_ascii() {
        return Cow::Borrowed(s);
    }
    match map(s) {
        Some(v) => Cow::Owned(v),
        // 系统没装中文 NLS 数据 / API 失败：原样返回，宁可显示简体也不能吞字
        None => Cow::Borrowed(s),
    }
}

fn map(s: &str) -> Option<String> {
    let src: Vec<u16> = s.encode_utf16().collect();
    // 逐字映射，目标长度恒等于源长度；多留一格避免个别映射变长的边界情况
    let mut dst = vec![0u16; src.len() + 1];
    let n = unsafe {
        LCMapStringEx(
            LOCALE,
            LCMAP_TRADITIONAL_CHINESE,
            &src,
            Some(&mut dst),
            None,
            None,
            Default::default(),
        )
    };
    if n <= 0 {
        return None;
    }
    dst.truncate(n as usize);
    String::from_utf16(&dst).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_is_untouched() {
        assert!(matches!(to_traditional("vhsj"), Cow::Borrowed("vhsj")));
        assert_eq!(to_traditional("，。"), "，。");
    }

    #[test]
    fn converts_common_chars() {
        // 只断言「变了且长度没变」——具体映射由系统字表决定，
        // 钉死某个字会在换 Windows 版本时误报
        let out = to_traditional("设置");
        assert_ne!(out.as_ref(), "设置", "设置 应该被转成繁体");
        assert_eq!(out.chars().count(), 2);
        assert_eq!(out.as_ref(), "設置");
    }

    #[test]
    fn is_idempotent() {
        let once = to_traditional("指示器按钮后面").into_owned();
        let twice = to_traditional(&once).into_owned();
        assert_eq!(once, twice, "繁体再转一次不应该继续变");
    }

    #[test]
    fn keeps_length_for_mixed_text() {
        let src = "第1个abc指示器";
        let out = to_traditional(src);
        assert_eq!(out.chars().count(), src.chars().count());
    }
}
