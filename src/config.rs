//! 壳侧的 `config` 门面。
//!
//! `Config` 本身、解析、迁移、原子写盘全在 `feihe_core::config` 里（那边才是真正
//! 的实现，有 16 项单测跟着搬过去了）。这里只做一件事：**给 `dir()` 补上「测试
//! 进程」这一档**。
//!
//! ## 为什么非得有这么一层
//!
//! 数据目录的策略由壳注册（`lib.rs` 的 `DllMain` 调
//! `paths::set_resolver(datadir::resolve)`），但**单元测试的产物是个 exe，`DllMain`
//! 根本不会执行**。没有注册就直接 `paths::dir()` 会 panic——这不是 bug，是
//! `paths` 刻意的设计（宁可炸在第一次用，也不要静默写到 `/` 下面）。
//!
//! 所以测试进程在这里明确走 `%APPDATA%\Feihe`——和拆 crate 之前
//! `resolve_dir` 里那句 `if cfg!(test) { return fallback }` 一模一样的行为。
//! 把判断放在这一层而不是 core 里，是因为只有这里 `cfg!(test)` 才真的表示
//! 「正在跑壳的测试」；core 被当依赖编进来时它永远是 false。
//!
//! 副作用要清楚：测试写的是**用户真实的数据目录**。所以任何会落盘的测试都必须
//! 用临时文件名、跑完删掉，不能覆盖 `config.ini` / `freq.txt`。

pub use feihe_core::config::*;

use std::path::PathBuf;

/// 取数据目录。生产进程交给 core 的注入机制，测试进程固定走 `%APPDATA%\Feihe`。
#[cfg(not(test))]
pub fn dir() -> PathBuf {
    feihe_core::paths::dir()
}

#[cfg(test)]
pub fn dir() -> PathBuf {
    crate::datadir::legacy_dir()
}

/// 测试用的全局锁，**顺便**把数据目录注册进去。
///
/// 光有 [`dir`] 那一层还不够：core 里的 `config::load` / `save` 走的是
/// core 自己的 `path()`，它们不会经过这个门面。所以测试进程必须在**第一次**
/// 碰配置之前把解析器注册好——把所有会碰配置的测试都要拿的那把锁拿来当锚点，
/// 一次注册、后面全命中（`set_resolver` 幂等）。
///
/// 这也是为什么这个函数不能只放在 core 里：`%APPDATA%` 是 Windows 的知识，
/// core 不该知道。
#[cfg(test)]
pub fn test_guard() -> std::sync::MutexGuard<'static, ()> {
    static L: std::sync::Mutex<()> = std::sync::Mutex::new(());
    feihe_core::paths::set_resolver(crate::datadir::resolve);
    L.lock().unwrap_or_else(|e| e.into_inner())
}
