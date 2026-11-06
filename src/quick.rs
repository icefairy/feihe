//! 快捷输入（`/rq` `/sj` `/nl` …）的**壳侧门面**。
//!
//! 实现整个搬到了 [`feihe_core::quick`]：那个模块现在一行系统调用都没有
//! （时间走 `core::clock`、UUID 自己按 v4 形状拼、剪贴板走 `fancha` 注入的读取器），
//! 所以 Windows 和 Android 用的是同一份命令表——「手机上 `/rq` 打不出日期」
//! 这种两端不一致不会再出现。
//!
//! 这里只做 `pub use`，`textservice` 里的 `quick::PREFIX` 等用法一行不用改。
//! 真正的文档（为什么用 `/` 当引导符、为什么 `/c` 不叫 `/v`）在 core 那个文件头上。

pub use feihe_core::quick::{candidates, code_ok, commands, EXPR_KEY, PREFIX};
