//! 全屏 / 演示场景检测：决定自绘的候选窗、状态浮窗「现在该不该弹」。
//!
//! ## 为什么用 `SHQueryUserNotificationState`
//!
//! 这个 API 是 shell 给「通知」用的：它知道当前用户会话是不是被一个全屏 D3D
//! 程序、演示模式、或者全屏 UWP 应用占着。要的正是这件事，而且判据是**全系统
//! 共享的一份状态**，不是我们自己拿窗口矩形去猜。
//!
//! 拿 `GetForegroundWindow` + `GetWindowRect` 比屏幕尺寸也能得到类似结果，
//! 但误判太多：最大化窗口、无边框窗口、多显示器跨屏、每用户不同 DPI 全都会偏。
//! 猜错的代价不是「少弹一个窗」——用户在全屏游戏里被弹了候选窗，游戏可能直接
//! 最小化掉。所以宁可借 shell 的判断。
//!
//! ## 为什么不是每次都问
//!
//! `no_popup()` 在每个按键上会被问一两次（候选窗 + 浮窗）。`SHQueryUserNotificationState`
//! 是跨进程的 shell 调用，虽然不慢，但打字是最高频的动作，没必要。这里缓存
//! 400ms：切到全屏再到下一次按键，跨度远大于这个值，不会漏判；连续打字时也就
//! 每 400ms 问一次。
//!
//! ## 判据只取三种
//!
//! - `QUNS_RUNNING_D3D_FULL_SCREEN`：独占全屏的 D3D 程序，典型就是游戏
//! - `QUNS_PRESENTATION_MODE`：投影 / 演示模式（别人的屏幕上不该冒出候选窗）
//! - `QUNS_APP`：全屏的 UWP 应用（商店版游戏、视频 App）
//!
//! **不算 `QUNS_BUSY`**：那是 UAC 提权框、系统模态对话框，那种时候根本轮不到
//! 我们打字，把它算进来反而会让「提权对话框挂后台」的用户突然看不到候选窗。

use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::Win32::UI::Shell::SHQueryUserNotificationState;

/// 缓存有效期。见模块注释。
const TTL: Duration = Duration::from_millis(400);

static CACHED: Mutex<Option<(Instant, bool)>> = Mutex::new(None);

/// shell 的会话状态里，哪几种意味着「别弹自绘窗口」。
///
/// 抽成纯函数是为了单测——`SHQueryUserNotificationState` 在测试进程里能不能
/// 调通取决于跑测试的会话，不能拿来断言。
fn covers_popup(state: i32) -> bool {
    // 3 = QUNS_RUNNING_D3D_FULL_SCREEN, 4 = QUNS_PRESENTATION_MODE, 7 = QUNS_APP
    matches!(state, 3 | 4 | 7)
}

/// 当前是否处在该收起自绘窗口的场景。查询失败按「不该收起」处理。
pub fn active() -> bool {
    if let Ok(g) = CACHED.lock() {
        if let Some((at, v)) = *g {
            if at.elapsed() < TTL {
                return v;
            }
        }
    }
    // SAFETY: 无参数、无指针，只读 shell 的会话状态。失败返回 Err，已兜住。
    let state = unsafe { SHQueryUserNotificationState() }
        .map(|s| s.0 as i32)
        .unwrap_or(0);
    let v = covers_popup(state);
    if let Ok(mut g) = CACHED.lock() {
        *g = Some((Instant::now(), v));
    }
    v
}

/// 丢掉缓存。切输入法（`Activate`）时调一次，用户刚从一个全屏游戏切回来时
/// 不必等 TTL 过期。
pub fn invalidate() {
    if let Ok(mut g) = CACHED.lock() {
        *g = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 全屏三兄弟算「收起」，其余一律不算。
    ///
    /// 0（未初始化）和 1（锁屏 / 无人）必须落在「不收起」这边：前者是查询失败，
    /// 后者在锁屏时本来就轮不到输入法，按不收起处理最安全。
    #[test]
    fn only_fullscreen_states_count() {
        assert!(covers_popup(3), "D3D 全屏");
        assert!(covers_popup(4), "演示模式");
        assert!(covers_popup(7), "全屏 UWP");
        assert!(!covers_popup(0));
        assert!(!covers_popup(1));
        assert!(!covers_popup(2), "QUNS_BUSY 不算，见模块注释");
        assert!(!covers_popup(5), "正常接受通知");
        assert!(!covers_popup(6));
    }
}
