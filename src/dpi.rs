//! DPI 感知：把代码里写的「逻辑像素」换算成当前显示器上的物理像素
//!
//! ## 为什么必须有这一层
//!
//! 我们是个 **TSE（输入法 DLL）**，自己画候选窗、状态浮窗、符号面板和设置窗。
//! 这些窗口的尺寸在代码里是写死的整数（`ROW_H = 28`、`CELL_W = 36`、
//! 设置窗 `W = 460`）——在 96 DPI 的屏上它们刚刚好，到了 150% / 200% 缩放的
//! 笔记本上就变成「一行候选 28 物理像素」，字是 13px 而窗口也按 13px 算，
//! 相对系统里其他界面小一半，看着像贴纸、点起来费劲。
//!
//! ## 为什么不去调 `SetProcessDpiAwareness`
//!
//! 我们不是 exe，是**进程内的 COM 组件**。DPI 感知是**进程级**状态，宿主
//! （资源管理器、记事本、Chrome……）在加载我们之前就已经定好了，我们从 DLL 里
//! 改它会把宿主整个界面的缩放打乱——那是不能碰的东西。
//!
//! 所以这里只做一件事：**问系统「你现在给我的是多少 DPI」，然后按比例放大自己的
//! 度量**。感知模式由宿主决定，我们跟随。
//!
//! ## unaware 宿主
//!
//! 如果宿主是 DPI unaware（老程序），系统会把我们的窗口连同坐标一起「虚拟化」放大，
//! 我们**再**按 DPI 放大就是双重放大。所以 `awareness()` 不是 PER_MONITOR /
//! SYSTEM 一律按 96 处理，见 [`effective`]。
//!
//! ## 缓存
//!
//! 命中判定（鼠标坐标 → 第几个候选）和绘制必须用同一套几何，而它们每帧都跑。
//! 每次按键都去问一遍 DPI 是没必要的跨进程调用（`GetDpiForMonitor` 会问 DWM），
//! 所以缓存一个 [`DPI`]，在窗口创建 / `WM_DPICHANGED` / 显示到新光标位置时刷新。

use std::sync::atomic::{AtomicU32, Ordering};

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::HiDpi::{
    GetAwarenessFromDpiAwarenessContext, GetDpiForMonitor, GetDpiForSystem, GetDpiForWindow,
    GetThreadDpiAwarenessContext, DPI_AWARENESS_UNAWARE, MDT_EFFECTIVE_DPI,
};

/// 100% 缩放时的 DPI。所有逻辑尺寸都是按它写的。
pub const BASE: u32 = 96;

static DPI: AtomicU32 = AtomicU32::new(BASE);

/// 按 DPI 缩放一个逻辑像素值。
///
/// 抽成纯函数是为了能单测：`scale(13, 120) == 16` 这种事，只有写成纯函数
/// 才能在没接显示器的 CI 上验证。
///
/// 四舍五入（`+ BASE/2`）而不是直接截断：截断会让每个尺寸都小半像素，
/// 一行 9 个候选攒下来能差出 4px，最后那个候选被挤掉一点。
pub fn scale(v: i32, dpi: u32) -> i32 {
    if v == 0 {
        return 0;
    }
    // 0 是「还没问过系统」的哨兵值，不是真的 0 DPI——按 100% 处理，
    // 否则所有尺寸都会被缩成 1px。
    let dpi = if dpi == 0 { BASE } else { dpi } as i32;
    let base = BASE as i32;
    let scaled = (v * dpi + base / 2) / base;
    // 逻辑上非零的量不能缩成 0：「1px 边框」在被截断成 0 之后就整条消失了
    if scaled == 0 {
        v.signum()
    } else {
        scaled
    }
}

/// 当前缓存的 DPI。
pub fn dpi() -> u32 {
    DPI.load(Ordering::Acquire)
}

/// 把逻辑像素换算成当前 DPI 下的物理像素。
pub fn px(v: i32) -> i32 {
    scale(v, dpi())
}

/// 把物理像素换算回逻辑像素。
///
/// 窗口创建之后，客户区尺寸、子窗口位置这些数字都是**系统给的物理值**，
/// 而布局代码（`build()` 里那一串 `y += 40`）全按逻辑值算。两者混着用就会出现
/// 「150% 缩放下内容区算出来比实际矮 40%」，滚到底也看不见最后几行。
/// 所以凡是「从系统读回来的尺寸」都先过一遍这里，统一回到逻辑坐标系。
pub fn unscale(v: i32, dpi: u32) -> i32 {
    let d = if dpi == 0 { BASE } else { dpi } as i32;
    let base = BASE as i32;
    if v == 0 {
        return 0;
    }
    let out = (v * base + d / 2) / d;
    if out == 0 {
        v.signum()
    } else {
        out
    }
}

fn store(d: u32) {
    DPI.store(effective(d), Ordering::Release);
}

/// 宿主进程的 DPI 感知模式不是 unaware 时才认系统报上来的 DPI。
///
/// unaware 的宿主里，系统会把我们的窗口和坐标一起虚拟化放大（我们交出的是
/// 逻辑坐标，它负责乘），这时再按真实 DPI 放大就是双重放大。
fn effective(d: u32) -> u32 {
    if d == 0 {
        return BASE;
    }
    if aware() {
        d
    } else {
        BASE
    }
}

/// 当前线程的 DPI 感知模式。unaware 时返回 false。
fn aware() -> bool {
    unsafe {
        let ctx = GetThreadDpiAwarenessContext();
        if ctx.0.is_null() {
            // 老系统（Win8.1 之前）没有这套 API，当作 unaware 最安全
            return false;
        }
        GetAwarenessFromDpiAwarenessContext(ctx) != DPI_AWARENESS_UNAWARE
    }
}

/// 按窗口所在显示器刷新（窗口已存在时首选这个）。
pub fn update_for_window(hwnd: HWND) {
    if hwnd.0.is_null() {
        return;
    }
    let d = unsafe { GetDpiForWindow(hwnd) };
    store(if d == 0 { unsafe { GetDpiForSystem() } } else { d });
}

/// 按屏幕坐标 `(x, y)` 所在的显示器刷新。
///
/// 候选窗是「光标在哪弹在哪」，跨屏时必须按**光标那块屏**算——主屏 150%、
/// 副屏 100% 是常见配置（笔记本 + 外接显示器），按主屏算会让副屏上的候选
/// 大出一半。
pub fn update_for_point(x: i32, y: i32) {
    let d = unsafe {
        let pt = POINT { x, y };
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        if mon.is_invalid() {
            GetDpiForSystem()
        } else {
            let (mut dx, mut dy) = (0u32, 0u32);
            if GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy).is_ok() && dx != 0 {
                dx
            } else {
                GetDpiForSystem()
            }
        }
    };
    store(d);
}

/// 按主屏刷新。窗口还没建、也没有坐标可依时的兜底。
pub fn update_for_system() {
    store(unsafe { GetDpiForSystem() });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 100% 缩放必须原样返回——它是所有既有布局的基准，动了就全歪。
    #[test]
    fn scale_is_identity_at_100_percent() {
        for v in [1, 5, 13, 28, 240, 460] {
            assert_eq!(scale(v, 96), v, "{v} 在 96 DPI 下不该变");
        }
    }

    /// 125% / 150% / 200% 按四舍五入放大。
    #[test]
    fn scale_rounds_at_common_ratios() {
        assert_eq!(scale(13, 120), 16, "13 * 1.25 = 16.25 → 16");
        assert_eq!(scale(28, 120), 35, "28 * 1.25 = 35");
        assert_eq!(scale(13, 144), 20, "13 * 1.5 = 19.5 → 20");
        assert_eq!(scale(36, 144), 54);
        assert_eq!(scale(28, 192), 56, "200% 正好翻倍");
        assert_eq!(scale(13, 192), 26);
    }

    /// 非零的量不能被缩成 0：1px 的边框没了，窗口看上去就没有边界。
    #[test]
    fn scale_never_floors_nonzero_to_zero() {
        assert_eq!(scale(1, 96), 1);
        assert_eq!(scale(0, 144), 0);
        // 0 是「没问过系统」的哨兵：按 100% 处理，不能把 13 缩成 1
        assert_eq!(scale(13, 0), 13);
        // 1 DPI 是病态输入（真实显示器不存在），要求不崩、且至少留 1px
        assert_eq!(scale(13, 1), 1);
    }

    /// `unscale` 是 `scale` 的逆运算：物理值换回逻辑值，供「从系统读回来的尺寸」
    /// 进入逻辑坐标系（设置窗的 `want_client_h` 就靠它把屏幕高度折回逻辑值）。
    #[test]
    fn unscale_inverts_scale() {
        for dpi in [96, 120, 144, 192] {
            for v in [1, 13, 28, 460, 820] {
                let phys = scale(v, dpi);
                let back = unscale(phys, dpi);
                // 四舍五入只在两端各丢半像素，来回一趟最多差 1
                assert!(
                    (back - v).abs() <= 1,
                    "{v} → {phys} → {back}（dpi={dpi}）偏得太多"
                );
            }
        }
        assert_eq!(unscale(0, 144), 0);
        // 0 DPI 是「还没问过系统」的哨兵，按 100% 处理
        assert_eq!(unscale(460, 0), 460);
    }

    /// 缓存只在真机上被写；单测里**绝不**去动它。
    ///
    /// `candidate` / `softkbd` 的几何用例全都按 96 DPI 的常数断言，而测试默认
    /// 并行执行——任何一个用例往全局缓存里写个 144，别人的断言就会假失败，
    /// 而且失败得毫无规律。所以这里只验证 `px()` 确实走的是当前缓存。
    #[test]
    fn px_uses_current_cache() {
        let d = dpi();
        assert!(d >= BASE, "缓存里不该出现比 96 还小的 DPI：{d}");
        assert_eq!(px(13), scale(13, d));
        assert_eq!(px(0), 0);
    }
}
