//! 简繁转换：中文模式下「繁体」开关的实现。
//!
//! ## 映射表由壳提供
//!
//! 常见做法是塞一张 `s2t.txt`（三四千对，UTF-8 约 30 KB）再二分查表。Windows 上
//! 改用系统自带的 NLS 映射表（`LCMapStringEx` + `LCMAP_TRADITIONAL_CHINESE`）：
//! 零体积、零构建步骤、系统升级时字表跟着更新。但那套 API 只有 Windows 有，
//! 所以**映射本身留在壳里**（`src/nls.rs`），core 通过 `set_converter` 拿一个
//! 函数指针，和 `paths::set_resolver` / `fancha::set_clipboard_reader` 同一个套路。
//!
//! 这样切分之后 core 里仍然有值得测的东西：开关（选简体就原样返回）、ASCII 快路径、
//! 转换失败时兜底不吞字。**「设」到底映射成什么**由壳的测试负责——那本来就是
//! Windows NLS 的行为，跟 Android 无关。
//!
//! 未注册转换器时（测试进程、还没做简繁的 Android 壳）一律原样返回。
//!
//! ## 转换粒度
//!
//! 逐字映射，不做词组消歧（「头发」→「頭髮」而不是「頭发」是靠 NLS 表里
//! 那个字本身的多对一关系，不是上下文判断）。搜狗/微软拼音也是这个粒度，
//! 对输入法足够——真正需要词组级消歧的场合用户会自己改。

use std::borrow::Cow;
use std::sync::OnceLock;

/// 简体 → 繁体。失败（系统没有中文 NLS 数据、API 出错）返回 `None`，
/// 调用方据此原样输出——**绝不因为转换失败丢字**。
pub type Converter = fn(&str) -> Option<String>;

static CONVERTER: OnceLock<Converter> = OnceLock::new();

/// 由壳在启动时注册。重复注册无效（第一次生效）。
pub fn set_converter(f: Converter) {
    let _ = CONVERTER.set(f);
}

/// 按当前设置转换：**只在「字形 = 繁体」时才动手**。
///
/// 所有调用点都必须走这个函数，不要直接调 `to_traditional`——后者是「无条件转」，
/// 早期版本就是因为四个调用点全都直接调它、谁也没看配置，导致选了简体也照样出繁体。
/// 把开关收在入口里，调用点就不需要（也不可能忘记）重复判断。
pub fn apply(s: &str) -> Cow<'_, str> {
    if !crate::config::get().traditional {
        return Cow::Borrowed(s);
    }
    to_traditional(s)
}

/// 简体 → 繁体（**无条件**，不看配置）。
///
/// 纯 ASCII（编码、标点、数字）直接原样返回，不做任何转换——这条快路径覆盖了
/// 绝大多数调用，因为组合串和候选里的短词经常是纯字母/纯标点。
///
/// 对已经是繁体的文本再转一次是幂等的（繁体字映射到自身），所以「候选窗转一遍、
/// 上屏时再转一遍」不会出问题。
pub fn to_traditional(s: &str) -> Cow<'_, str> {
    if s.is_empty() || s.is_ascii() {
        return Cow::Borrowed(s);
    }
    match CONVERTER.get().and_then(|f| f(s)) {
        Some(v) => Cow::Owned(v),
        // 没注册转换器 / 映射失败：原样返回，宁可显示简体也不能吞字
        None => Cow::Borrowed(s),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 只认「设」的假转换器。
    ///
    /// core 的单测**不能**依赖真实字表：Windows 上是 NLS，Android 上将来可能是
    /// 另一张表，拿真表断言会让「换个平台就红」。这里只验证四件事：注册了就会用、
    /// 配置开关能拦住、ASCII 走快路径、失败不吞字。
    fn fake(s: &str) -> Option<String> {
        Some(s.replace('设', "設"))
    }

    /// `OnceLock` 只认第一次，所以每个用例都调一遍是安全的。
    fn install() {
        set_converter(fake);
    }

    /// 和 textservice / dict 的用例共用一把锁：`config` 是进程级全局状态
    fn lock() -> std::sync::MutexGuard<'static, ()> {
        crate::dict::scheme_guard()
    }

    #[test]
    fn apply_is_noop_when_simplified() {
        let _g = lock();
        install();
        crate::config::update(|c| c.traditional = false);
        // 选简体就必须原样输出——这是用户报的 bug：选了简体却打出繁体
        assert_eq!(apply("设置").as_ref(), "设置");
        assert!(matches!(apply("设置"), Cow::Borrowed(_)));
        crate::config::update(|c| c.traditional = false);
    }

    #[test]
    fn apply_converts_when_traditional() {
        let _g = lock();
        install();
        crate::config::update(|c| c.traditional = true);
        assert_eq!(apply("设置").as_ref(), "設置");
        crate::config::update(|c| c.traditional = false);
    }

    #[test]
    fn ascii_is_untouched() {
        install();
        // 快路径：连转换器都不该被调用
        assert!(matches!(to_traditional("vhsj"), Cow::Borrowed("vhsj")));
        assert_eq!(to_traditional("，。"), "，。");
    }

    #[test]
    fn converts_and_keeps_length() {
        install();
        let out = to_traditional("设置");
        assert_ne!(out.as_ref(), "设置", "「设置」应该被转成繁体");
        assert_eq!(out.chars().count(), 2);
        let src = "第1个abc设置";
        assert_eq!(to_traditional(src).chars().count(), src.chars().count());
    }

    #[test]
    fn is_idempotent() {
        install();
        let once = to_traditional("指示器按设").into_owned();
        let twice = to_traditional(&once).into_owned();
        assert_eq!(once, twice, "繁体再转一次不应该继续变");
    }
}
