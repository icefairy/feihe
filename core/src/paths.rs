//! 数据目录的唯一出处。
//!
//! 词频、联想、自造词、置顶、降权、自定义短语……最后都要落成「一个目录下的几个
//! 文本文件」。这些模块要能搬进 `feihe-core`，就不能自己去问 `%APPDATA%` 或
//! `Context.getFilesDir()`——那正是壳该决定的事。
//!
//! 所以这里只存一个 `PathBuf`：
//!
//! - **壳**在启动时调一次 [`init`]，告诉 core「数据放这儿」。
//! - **模块**通过 [`dir`] 拿路径，不关心它在哪。
//!
//! ## 为什么是 `OnceLock`，不是每次问壳
//!
//! `freq::file()` 在热路径上——每上屏一次就要拼一次路径。每次跨 FFI 问一次
//! 是几十纳秒起步的浪费，而且会让「同一个进程里路径会变」这种假想状态泄漏到
//! 所有调用点。数据目录在一个进程生命周期内是常量，就按常量存。
//!
//! ## 为什么没有单测
//!
//! `OnceLock` 一旦被写就不可逆，而 `cargo test` 是**多线程共享一个进程**：
//! 任何一个测试先 `init()` 了，后面的测试就再也测不到「未初始化」的分支，
//! 断言会随执行顺序飘。测这个要么加锁串行（把整个测试套拖慢），要么给它开个
//! 测试专用的重置口（生产代码里多一条只有测试走的路）。两条都不划算——
//! 这段代码的全部逻辑就是「存一次、取多次」，真正会错的是**忘了调用**，
//! 而那件事靠 [`dir`] 的 panic 消息在第一次运行时就能喊出来。

use std::path::PathBuf;
use std::sync::OnceLock;

static DIR: OnceLock<PathBuf> = OnceLock::new();

/// 壳注册的「数据目录该在哪」策略。一个进程只有一个，所以是普通函数指针。
///
/// 为什么要有这条，而不是让模块直接 `dir_or_else(自己的策略)`：`config` 本身
/// 已经搬进 core 了，它调不到壳里的 `resolve_dir`（那个要问 DLL 路径、要试写）。
/// 反过来让壳在启动时 `init(算好的路径)` 也不行——`resolve_dir` 会做文件 I/O
/// （试写探针、首次迁移），在 `DllMain` 里做文件 I/O 是 loader lock 大忌。
///
/// 所以拆成两半：**壳注册策略**（一次，纯赋值）＋ **core 首次用到时惰性执行**
/// （此时已远离 loader lock）。行为和拆 crate 之前完全一致。
static RESOLVER: OnceLock<fn() -> PathBuf> = OnceLock::new();

/// 壳启动时注册数据目录策略。重复注册会被忽略（先到先得）。
pub fn set_resolver(f: fn() -> PathBuf) {
    let _ = RESOLVER.set(f);
}

/// 壳启动时调用，只生效一次（重复调用会被忽略，不会把已在用的路径换掉）。
///
/// Windows 侧在 `config::resolve_dir` 里算出「安装目录优先、不可写回落
/// `%APPDATA%`」；Android 侧会是 `Context.getFilesDir()`。**策略在壳里，
/// 存储在这里**——core 不猜平台。
pub fn init(dir: PathBuf) {
    let _ = DIR.set(dir);
}

/// 取数据目录。没初始化过就用 `first` 惰性算一次。
///
/// Windows 壳走这条：`dir_or_else(resolve_dir)`，行为和拆 crate 之前一模一样
/// （第一次调用时解析，之后恒定）。
pub fn dir_or_else(first: impl FnOnce() -> PathBuf) -> PathBuf {
    DIR.get_or_init(first).clone()
}

/// 取数据目录。优先级：显式 [`init`] > 壳注册的 [`set_resolver`] > panic。
///
/// 两条路是有分工的，不是冗余：
/// - Android 的目录在 `init` 时就是确定的（`Context.getFilesDir()`），直接给。
/// - Windows 的目录要「试写 + 首次迁移」才知道，**不能在 `DllMain` 里算**，
///   所以注册策略、等第一次真正要读写文件时再算。
///
/// 两个都没设置会在这里炸，消息直接说明要做什么——比后面某处
/// 「文件写到了 `/` 下面」好查得多。
pub fn dir() -> PathBuf {
    if let Some(d) = DIR.get() {
        return d.clone();
    }
    if let Some(f) = RESOLVER.get() {
        return DIR.get_or_init(f).clone();
    }
    panic!("数据目录未初始化：壳必须先调用 feihe_core::paths::init() 或 set_resolver()");
}
