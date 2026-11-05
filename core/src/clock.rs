//! 「现在几点」的唯一出处。
//!
//! `custom` 的自定义短语要把 `{year}` `{hour}` `{weekday}` 展开成当前时间，而
//! **本地时间是平台知识**：Windows 有 `GetLocalTime`，Android 的 Rust 侧拿不到
//! 时区（`std` 故意不提供本地时间）。
//!
//! 所以这里分两层：
//!
//! - **壳可以注册一个 provider**（Windows 直接包 `GetLocalTime`，最准，含夏令时）；
//! - **不注册时**退化成「`SystemTime` 的 UTC + 偏移分钟」，偏移由壳设置
//!   （Android 在启动时从 `TimeZone.getDefault()` 算一次）。
//!
//! 不注册也不设偏移时是 UTC。**不 panic**：拿不到本地时间最多是短语里的日期差
//! 8 小时，让整个输入法起不来是完全不成比例的代价。

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::OnceLock;

/// 一个本地时间的快照。字段含义和 Windows 的 `SYSTEMTIME` 一致——
/// 这个结构本来就是从它抽象出来的，`weekday` 0 表示周日。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct LocalTime {
    pub year: u16,
    pub month: u16,
    pub day: u16,
    /// 0 = 周日，1 = 周一 … 6 = 周六。
    pub weekday: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
}

static PROVIDER: OnceLock<fn() -> LocalTime> = OnceLock::new();

/// 和 UTC 的偏移，单位分钟。只在没有 provider 时用。
static UTC_OFFSET_MIN: AtomicI32 = AtomicI32::new(0);

/// 壳注册本地时间来源（Windows 用 `GetLocalTime`）。重复注册会被忽略。
pub fn set_provider(f: fn() -> LocalTime) {
    let _ = PROVIDER.set(f);
}

/// 壳告诉 core「本机时区比 UTC 快多少分钟」。
///
/// 给没有 `GetLocalTime` 的平台用（Android：Kotlin 那边
/// `TimeZone.getDefault().getOffset(now) / 60000`）。
/// 传进来的是**当前时刻**的偏移，夏令时切换后要重新设置——所以 Android 侧
/// 每次 `onStartInputView` 顺手刷一次，比在这里做时区库实在。
pub fn set_utc_offset_minutes(minutes: i32) {
    UTC_OFFSET_MIN.store(minutes, Ordering::Release);
}

/// 当前本地时间。
pub fn now() -> LocalTime {
    if let Some(f) = PROVIDER.get() {
        return f();
    }
    from_unix_seconds(
        unix_seconds() + i64::from(UTC_OFFSET_MIN.load(Ordering::Acquire)) * 60,
    )
}

fn unix_seconds() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        // 系统时间被调到 1970 之前：当作 epoch，别 panic。
        Err(_) => 0,
    }
}

/// Unix 秒 → 年月日时分秒 + 星期。纯算术，不依赖任何时区库。
///
/// 用的是 Howard Hinnant 的 `civil_from_days`（公有领域，被 C++20 的
/// `<chrono>` 采用）。自己写这一段是因为**不想为了一个日期分解引一个依赖**，
/// 而这段是纯整数运算，可以完整单测。
pub fn from_unix_seconds(secs: i64) -> LocalTime {
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);

    let (year, month, day) = civil_from_days(days);
    // 1970-01-01 是周四。0 = 周日，所以周四 = 4。
    let weekday = (days + 4).rem_euclid(7) as u16;

    LocalTime {
        year: year as u16,
        month: month as u16,
        day: day as u16,
        weekday,
        hour: (sod / 3600) as u16,
        minute: ((sod % 3600) / 60) as u16,
        second: (sod % 60) as u16,
    }
}

/// 天数（自 1970-01-01 起，可为负）→ (年, 月, 日)。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]，3 月为 0
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (y + i64::from(m <= 2), m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 拿几个已知时刻钉住换算。这些都对应到「人一眼能核对」的日子，
    /// 包括闰日和跨世纪——日期算法最容易错的就是这两个地方。
    #[test]
    fn unix_seconds_to_civil() {
        // 1970-01-01 00:00:00 UTC 是周四
        let t = from_unix_seconds(0);
        assert_eq!((t.year, t.month, t.day), (1970, 1, 1));
        assert_eq!((t.hour, t.minute, t.second), (0, 0, 0));
        assert_eq!(t.weekday, 4, "1970-01-01 是周四");

        // 2026-11-05 15:51:18 UTC（本次会话的时间，周四）
        let t = from_unix_seconds(1_793_893_878);
        assert_eq!((t.year, t.month, t.day), (2026, 11, 5));
        assert_eq!((t.hour, t.minute, t.second), (15, 51, 18));
        assert_eq!(t.weekday, 4);

        // 2024-02-29（闰日）
        let t = from_unix_seconds(1_709_164_800);
        assert_eq!((t.year, t.month, t.day), (2024, 2, 29));

        // 2000-03-01（世纪闰年之后一天）
        let t = from_unix_seconds(951_868_800);
        assert_eq!((t.year, t.month, t.day), (2000, 3, 1));

        // 1970 之前也要算得出来，不能 panic
        let t = from_unix_seconds(-86_400);
        assert_eq!((t.year, t.month, t.day), (1969, 12, 31));
    }

    /// 偏移是「加到 UTC 上」的分钟数，用来在没有 `GetLocalTime` 的平台
    /// （Android）把 UTC 换成当地墙上时间。
    #[test]
    fn utc_offset_shifts_the_clock() {
        // 不做全局设置（会和并行测试抢），直接验算术：
        // UTC 15:51 + 480 分钟 = 23:51
        let t = from_unix_seconds(1_793_893_878 + 480 * 60);
        assert_eq!((t.hour, t.minute), (23, 51));
        assert_eq!(t.day, 5, "跨小时不跨天");
    }
}
