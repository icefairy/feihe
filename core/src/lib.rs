//! 飞鹤输入法 · 平台无关的核心
//!
//! 这一层的规矩只有一条：**不许出现任何操作系统 API**。没有 `windows` crate、
//! 没有注册表、没有剪贴板、没有 TSF/UI 类型、没有线程本地存储。文件 IO 只碰
//! 数据目录（路径由壳注入），时钟/日志/剪贴板这类能力一律走钩子。
//! 同样的代码要能在 Windows 的 TSF 壳里跑，也要能在 Android 的
//! `InputMethodService` 壳里跑，还要能在测试里被反复构造。
//!
//! ## 为什么拆出来
//!
//! 原来这些模块都躺在 `src/` 下跟 TSF 代码混在一起，整个 crate 依赖 `windows`。
//! `windows` crate 在 `aarch64-linux-android` 上编不过，所以只要它们还挂在同一个
//! crate 里，Android 端就一行代码都复用不了——哪怕这些模块本身一个系统调用都不碰。
//!
//! 拆开之后，Windows 壳用 `pub use feihe_core::{…}` 把它们挂回 crate 根，所有
//! 既有的 `crate::shuangpin::…` 路径一个字都不用改。这是刻意的：搬家不该顺手
//! 改调用点，否则回归了分不清是「搬家搬坏了」还是「顺手改坏了」。
//!
//! ## 分批搬
//!
//! 一次只搬**真正的叶子**（不引用任何 Windows 侧模块的）。搬运顺序见
//! `docs/Android-方案.md` M1：先纯函数，后碰文件的，最后才是状态机
//! （`textservice` 拆成 `Session` 纯状态机 + 壳，那是最后一步，也是唯一有风险的一步）。

pub mod assist;
pub mod backup;
pub mod brief;
pub mod brief2;
pub mod calc;
pub mod clock;
pub mod config;
pub mod correct;
pub mod custom;
pub mod demote;
pub mod dict;
pub mod dictbin;
pub mod dictenc;
pub mod fancha;
pub mod freq;
pub mod fuzzy;
pub mod hint;
pub mod log;
pub mod next;
pub mod nextbin;
pub mod number;
pub mod paths;
pub mod phrase;
pub mod pin;
pub mod segment;
pub mod shuangpin;
pub mod zhconv;

/// core 的版本号，跟 `Cargo.toml` 走。
///
/// 给壳用：Android 那边要在设置页/日志里显示「引擎版本」，而它跟 APK 的
/// versionName 不一定是同一个数（APK 可以只换壳不换引擎）。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
