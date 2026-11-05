//! 简繁映射的 Windows 实现（`LCMapStringEx`）。
//!
//! 转换的**时机**在 core（`feihe_core::zhconv::apply` 看配置开关、拦 ASCII 快路径、
//! 失败兜底），转换的**数据**在这里——系统 NLS 表，只要 Windows 有中文区域数据就
//! 能用。分开之后 core 能在 Android 上编，而「设 → 設」这种具体映射仍然只在
//! 真 Windows 上被验证。
//!
//! 拿不到数据（极少数没装 zh-CN NLS 的精简系统）返回 `None`，core 那边会原样输出，
//! 宁可显示简体也不吞字。

use windows::core::PCWSTR;
use windows::Win32::Globalization::{LCMapStringEx, LCMAP_TRADITIONAL_CHINESE};

/// 优先用的 locale。文档要求 `LCMAP_TRADITIONAL_CHINESE` 必须配中文 locale，
/// 用 `LOCALE_NAME_SYSTEM_DEFAULT` 在英文系统上会失败，所以显式指定。
const LOCALE: PCWSTR = windows::core::w!("zh-CN");

/// 简体 → 繁体，逐字映射。失败返回 `None`。
pub fn to_traditional(s: &str) -> Option<String> {
    if s.is_empty() || s.is_ascii() {
        return Some(s.to_string());
    }
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

    /// 具体映射只在这里断言——这是 Windows NLS 的真实行为，core 那份单测用的是
    /// 假转换器（见 `feihe_core::zhconv` 的说明）。
    #[test]
    fn converts_common_chars() {
        assert_eq!(to_traditional("设置").as_deref(), Some("設置"));
    }

    #[test]
    fn keeps_length_for_mixed_text() {
        let src = "第1个abc指示器";
        let out = to_traditional(src).expect("NLS 应该能转");
        assert_eq!(out.chars().count(), src.chars().count());
    }

    #[test]
    fn is_idempotent() {
        let once = to_traditional("指示器按钮后面").expect("NLS 应该能转");
        let twice = to_traditional(&once).expect("NLS 应该能转");
        assert_eq!(once, twice, "繁体再转一次不应该继续变");
    }

    #[test]
    fn ascii_is_untouched() {
        assert_eq!(to_traditional("vhsj").as_deref(), Some("vhsj"));
        assert_eq!(to_traditional("，。").as_deref(), Some("，。"));
    }
}
