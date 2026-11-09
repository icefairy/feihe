//! 多显示器几何：某一点落在哪块屏上、窗口该夹进哪块屏。
//!
//! ## 为什么不能用 `SM_CXSCREEN` / `SM_CYSCREEN`
//!
//! 这两个数给的是**主屏**尺寸，而且屏幕坐标的原点固定在主屏左上角：
//! 主屏右侧的副屏 x 大于主屏宽，主屏左侧的副屏 x 是**负数**，副屏比主屏高时
//! y 也会超过主屏高度。拿主屏尺寸去夹窗口，副屏上的窗口就会被钳回主屏：
//!
//! - 左边副屏 (x = -1080..0) 打字：`px < 0` 被钳到 0，窗口跑到**主屏最左边**；
//!   用户看到的是「在 2 号屏打字，候选框出现在主屏边上」。
//! - 竖屏比主屏高（1920 > 1440）时，光标在 y = 1654：窗口被放到
//!   (0, 1654)——这个坐标在两块屏之间没有任何显示器的空白区，也就是
//!   「候选窗根本不出现」。
//!
//! 所以凡是「窗口跟着光标/浮窗走」的定位，都要先问「这个点在哪块屏上」，
//! 再按那块屏的范围夹。

use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITOR_DEFAULTTONEAREST, MONITORINFO,
};
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

/// 拿不到显示器信息时的兜底：主屏。
///
/// 只在极端失败（`MonitorFromPoint` 返回空、`GetMonitorInfoW` 失败）下走到。
fn primary() -> RECT {
    unsafe {
        RECT {
            left: 0,
            top: 0,
            right: GetSystemMetrics(SM_CXSCREEN),
            bottom: GetSystemMetrics(SM_CYSCREEN),
        }
    }
}

fn info_at_point(x: i32, y: i32) -> Option<MONITORINFO> {
    unsafe {
        let hmon = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        if hmon.0.is_null() {
            return None;
        }
        let mut mi = MONITORINFO {
            cbSize: core::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        GetMonitorInfoW(hmon, &mut mi).as_bool().then_some(mi)
    }
}

/// `(x, y)` 所在显示器的**工作区**（不含任务栏）。
///
/// 候选窗、符号面板这类「贴着光标弹」的窗口用这个：贴到任务栏上虽然不算出屏，
/// 但会被挡住半截。
pub fn work_at(x: i32, y: i32) -> RECT {
    info_at_point(x, y)
        .map(|mi| mi.rcWork)
        .unwrap_or_else(primary)
}

/// 把 `(x, y)` 处一个 `w × h` 的窗口夹进 `area`。
///
/// 纯函数，方便单测：真机上两块屏的坐标组合很难在 CI 里复现，而这里算错
/// 的症状（候选窗跑到另一块屏、或者干脆不显示）又恰恰是最难靠肉眼归因的。
///
/// 窗口比 area 还宽/高时贴左上角——宁可露出一部分，也不能推出屏幕外。
pub fn clamp_into(area: &RECT, x: i32, y: i32, w: i32, h: i32) -> (i32, i32) {
    (
        x.clamp(area.left, (area.right - w).max(area.left)),
        y.clamp(area.top, (area.bottom - h).max(area.top)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 主屏在右、竖屏在左（用户报过的真实布局）：
    /// 副屏 (x = -1080..0, y = 0..1920)，主屏 (0..2560, 0..1440)。
    fn left_portrait() -> RECT {
        RECT {
            left: -1080,
            top: 0,
            right: 0,
            bottom: 1920,
        }
    }

    fn primary_landscape() -> RECT {
        RECT {
            left: 0,
            top: 0,
            right: 2560,
            bottom: 1440,
        }
    }

    /// 左边副屏靠下：x 是负数，y 超过主屏高度。
    ///
    /// 老代码会算出 `px = 0`（主屏最左边）和 `py = 1654`（主屏下方没有显示器的
    /// 空白区）——前者是「候选框跑到主屏」，后者是「候选框看不见」。
    #[test]
    fn left_monitor_bottom_stays_on_left_monitor() {
        let (x, y) = clamp_into(&left_portrait(), -500, 1654, 600, 40);
        assert!(x < 0, "窗口被钳到主屏了：x={x}");
        assert_eq!(x, -600, "600 宽的窗口在 -1080..0 里，左边界是 -600");
        assert_eq!(y, 1654, "y 落在副屏范围内，不该动");
    }

    /// 副屏比主屏高：光标在 y = 1900 时窗口必须往上收，而不是被推到主屏下方。
    #[test]
    fn taller_secondary_monitor_pulls_window_up() {
        let (x, y) = clamp_into(&left_portrait(), -300, 1900, 400, 40);
        assert_eq!(x, -400, "右边不越过副屏右边界（0）");
        assert_eq!(y, 1880, "底边贴住 1920，不是被夹到 1440");
    }

    /// 主屏右边缘：老逻辑只夹右边，这里保持一致。
    #[test]
    fn right_edge_of_primary() {
        let (x, _) = clamp_into(&primary_landscape(), 2400, 100, 600, 40);
        assert_eq!(x, 1960);
    }

    /// 窗口比显示器还宽：贴左边界，不许出现负宽度的钳位区间。
    #[test]
    fn window_wider_than_monitor_pins_left() {
        let (x, y) = clamp_into(&left_portrait(), -300, 100, 1400, 40);
        assert_eq!(x, -1080);
        assert_eq!(y, 100);
    }
}
