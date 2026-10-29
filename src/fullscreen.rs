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

static CACHED: Mutex<Option<(Instant, Scene)>> = Mutex::new(None);

/// 当前处在哪种「吃满屏幕」的场景。
///
/// **为什么要分这么细**：这三种场景对输入法的要求完全不同。
///
/// - `Presentation`（投影 / 演示）：屏幕在给别人看，冒一个候选窗上去很尴尬，而且
///   演示时基本不打字 → **一个窗口都不弹**，候选写进组合串。
/// - `Game` / `Uwp`（全屏游戏 / 全屏商店应用）：用户就是在里面打字，**必须能看见
///   候选**。这类程序绝大多数是「无边框窗口化全屏」（Windows 10 起 D3D 的独占全屏
///   也由 DWM 的全屏优化接管），自绘窗口盖得上去；把它们也归进「不许弹」，用户
///   看到的就是「能敲键盘、屏幕上什么都没有」——等于输入法坏了。
/// - `Normal`：照常。
///
/// 早期只有一个布尔量（`fullscreen::active()`），上面两类被揉成一类，于是全屏游戏
/// 里候选窗被拦掉、组合串又不显示（游戏自绘 UI 不画 IME 组合串），彻底没法用。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scene {
    Normal,
    /// `QUNS_RUNNING_D3D_FULL_SCREEN`
    Game,
    /// `QUNS_PRESENTATION_MODE`
    Presentation,
    /// `QUNS_APP`：全屏 UWP
    Uwp,
}

impl Scene {
    /// 场景的名字，写日志用。
    pub fn key(self) -> &'static str {
        match self {
            Scene::Normal => "normal",
            Scene::Game => "game",
            Scene::Presentation => "presentation",
            Scene::Uwp => "uwp",
        }
    }

    /// 这个场景下**状态浮窗**该不该出现。
    ///
    /// 浮窗是常驻窗口（一直挂在屏幕上），全屏游戏里它会一直压在画面上挡视线，
    /// 所以任何全屏场景都不显示。注意这和「候选窗」是两件事：候选是用户敲了键
    /// 才出现、松手就消失的，全屏游戏里恰恰必须有。
    pub fn hides_statusbar(self) -> bool {
        self != Scene::Normal
    }
}

/// shell 的会话状态码 → 场景。
///
/// 抽成纯函数是为了单测——`SHQueryUserNotificationState` 在测试进程里能不能
/// 调通取决于跑测试的会话，不能拿来断言。
fn scene_of(state: i32) -> Scene {
    match state {
        3 => Scene::Game,
        4 => Scene::Presentation,
        7 => Scene::Uwp,
        _ => Scene::Normal,
    }
}

/// 这个场景要不要把**候选窗**收起来（只有演示模式才收）。
///
/// 见 `Scene` 的注释：全屏游戏里收起候选窗等于让输入法没法用。
///
/// `config::no_popup()` 的 Auto 档走的是这个函数——「什么时候该收候选窗」
/// 只能有一个出处，否则两边迟早漂移（症状是设置里写「演示时收起」、
/// 实际游戏里也收）。
pub(crate) fn covers_popup(scene: Scene) -> bool {
    scene == Scene::Presentation
}

/// 当前场景。查询失败按 `Normal` 处理。
pub fn scene() -> Scene {
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
    let v = scene_of(state);
    if let Ok(mut g) = CACHED.lock() {
        *g = Some((Instant::now(), v));
    }
    v
}

/// 是否处在任何「吃满屏幕」的场景（游戏 / 演示 / 全屏 UWP）。
pub fn active() -> bool {
    scene() != Scene::Normal
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

    /// 状态码到场景的映射。0（未初始化）和 1（锁屏 / 无人）都算 `Normal`：
    /// 前者是查询失败，后者在锁屏时本来就轮不到输入法。
    #[test]
    fn states_map_to_scenes() {
        assert_eq!(scene_of(3), Scene::Game, "D3D 全屏");
        assert_eq!(scene_of(4), Scene::Presentation, "演示模式");
        assert_eq!(scene_of(7), Scene::Uwp, "全屏 UWP");
        assert_eq!(scene_of(0), Scene::Normal);
        assert_eq!(scene_of(1), Scene::Normal);
        assert_eq!(scene_of(2), Scene::Normal, "QUNS_BUSY 不算，见模块注释");
        assert_eq!(scene_of(5), Scene::Normal, "正常接受通知");
        assert_eq!(scene_of(6), Scene::Normal);
    }

    /// **只有演示模式才收候选窗。** 全屏游戏收起来等于让输入法没法用——
    /// 用户反馈过「全屏游戏里能敲键盘但屏幕上什么都没有」，根因就是把游戏错并进了
    /// 「收起」那一类。
    #[test]
    fn only_presentation_hides_the_candidate_window() {
        assert!(covers_popup(Scene::Presentation));
        assert!(!covers_popup(Scene::Game), "全屏游戏里候选窗必须有");
        assert!(!covers_popup(Scene::Uwp), "全屏 UWP 同理");
        assert!(!covers_popup(Scene::Normal));
    }

    /// 状态浮窗反过来：**任何全屏场景都不显示**（它是常驻窗口，会一直挡画面）。
    #[test]
    fn every_fullscreen_scene_hides_the_statusbar() {
        assert!(Scene::Game.hides_statusbar());
        assert!(Scene::Presentation.hides_statusbar());
        assert!(Scene::Uwp.hides_statusbar());
        assert!(!Scene::Normal.hides_statusbar());
    }
}
