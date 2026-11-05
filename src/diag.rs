//! 运行时诊断日志：写到安装目录的 `feihe.log`。
//!
//! ## 为什么需要它
//!
//! 输入法跑在**别人的进程**里（记事本、浏览器、游戏），出问题时没有任何可见的
//! 错误。用户能提供的只有三句话：「没有候选窗」「字母直接进文档」「浮窗也不见了」。
//! 而这三句话背后至少有四种完全不同的成因：
//!
//! - `config::no_popup()` 判成「该收起」（全屏 / 演示 / 用户设成不弹）
//! - 宿主声明 UILess，候选交给它画，它没画（见 `uielement`）
//! - 密码框判断误命中，连组合都不开始
//! - 按键通道压根没挂上（`AdviseKeyEventSink` 失败 / compartment 被写成 0）
//!
//! 这四种从外面看一模一样。不记日志就只能猜，一猜就是几轮来回。
//!
//! ## 开销
//!
//! - 只记**生命周期**和**状态变化**（每种拒绝原因只记第一次），不在每个按键上写盘；
//! - 文件到 256 KB 就停写，不会把磁盘写满；
//! - 任何一步失败都静默放弃——诊断绝不许影响输入本身。
//!
//! 日志位置就是安装目录，和 `config.ini` 放一起，用户找得到也删得掉。

/// 写日志和「只在变化时写日志」都搬进了 `feihe_core::log`——因为 `freq` /
/// `next` / `dict` 这些要进 core 的模块都得记日志，而「日志写哪个文件」取决于
/// 数据目录（`paths`），跟平台无关。这里只留**平台相关的那一条**：前台窗口类名。
pub use feihe_core::log::{log, log_change};

/// 前台窗口类名。用来确认「这次激活到底发生在哪个程序里」——
/// 「记事本没候选」和「浏览器没候选」是两回事。
pub fn fg_class() -> String {
    use windows::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetForegroundWindow};
    unsafe {
        let h = GetForegroundWindow();
        if h.0.is_null() {
            return "-".into();
        }
        let mut buf = [0u16; 256];
        let n = GetClassNameW(h, &mut buf);
        if n <= 0 {
            return "?".into();
        }
        String::from_utf16_lossy(&buf[..n as usize])
    }
}

/// 写日志的简写。`#[macro_export]` 放在 crate 根，用 `crate::dlog!` 调。
#[macro_export]
macro_rules! dlog {
    ($($arg:tt)*) => {
        $crate::diag::log(format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! dlog_change {
    ($slot:expr, $code:expr, $($arg:tt)*) => {
        $crate::diag::log_change(&$slot, $code, format_args!($($arg)*))
    };
}
