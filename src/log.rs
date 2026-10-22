//! 临时诊断日志
//!
//! 排查「切到飞鹤拼音后按键直接进应用、不出候选窗」这类问题用。
//! 输出到 `%TEMP%\feihe-trace.log`，定位完就删掉本模块。
//!
//! 注意：不要在 `DllMain` 里调用——那里持有 loader lock，文件 IO 可能死锁。

use std::io::Write;

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

pub fn line(msg: &str) {
    let dir = std::env::var("TEMP").unwrap_or_else(|_| "C:\\Windows\\Temp".to_string());
    let path = format!("{dir}\\feihe-trace.log");
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let tid = std::thread::current().id();
    let _ = writeln!(
        f,
        "[{}][pid {}][{:?}] {}",
        now_ms(),
        std::process::id(),
        tid,
        msg
    );
}

#[macro_export]
macro_rules! trace {
    ($($arg:tt)*) => {
        $crate::log::line(&format!($($arg)*))
    };
}
