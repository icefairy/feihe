//! 状态浮窗：输入法的那条小横条
//!
//! 四个按钮：中/英、全/半、符号软键盘、设置。
//!
//! 皮肤和「全拼/双拼」曾经也在这里，后来挪进设置窗了——它们都是**低频、改了
//! 就长期不变**的东西，占着浮窗宽度不划算。浮窗只留「随时要按」的开关。
//!
//! 窗口是 `WS_POPUP | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST`，
//! 在宿主进程的 UI 线程上创建——和候选窗同一套路子。**不抢焦点**是硬要求：
//! 一旦抢焦点，宿主的文档会失焦，正在输入的组合串会被打断。
//!
//! 因为浮窗是 topmost 且存在于每个加载了本输入法的进程里，必须自己做「只在前台
//! 进程显示」的判断，否则多个应用各挂一条、全都飘在屏幕上。这里用一个 300ms 的
//! 定时器比对前台窗口的线程 id 来切换显隐。
//!
//! 拖动：按下后超过 3px 视为拖动（移动窗口并记住位置），否则在抬起时按按钮命中
//! 分发点击。这样一条窗口里既能拖又能点，不用额外的标题栏。

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicU8, Ordering};
use std::sync::Mutex;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config;
use crate::font;
use crate::insert;
use crate::settings;
use crate::softkbd;
use crate::theme;

const CLASS_NAME: PCWSTR = w!("FeiheStatusBar");

// ── 逻辑尺寸（96 DPI）─────────────────────────────────────────
//
// 和候选窗一样，这些数只是 100% 缩放下的基准，真机上要过 `dpi::px` 换成
// 物理像素（见 `src/dpi.rs`）。浮窗是常驻的，缩放下如果还按 96 算，
// 在 200% 的屏上会变成一条 30 物理像素的细缝，字挤成一团。
const PAD_L: i32 = 6;
const HEIGHT_L: i32 = 30;
/// 按钮上下各留的空，让高亮块不顶到边框
const BTN_INSET_L: i32 = 3;
/// 按钮宽度（逻辑像素）
const BTN_W: [i32; 4] = [34, 34, 44, 44];

fn pad() -> i32 {
    crate::dpi::px(PAD_L)
}
fn height() -> i32 {
    crate::dpi::px(HEIGHT_L)
}
fn btn_inset() -> i32 {
    crate::dpi::px(BTN_INSET_L)
}
/// 浮窗总宽：两侧留白 + 四个按钮
fn width() -> i32 {
    pad() * 2 + BTN_W.iter().map(|w| crate::dpi::px(*w)).sum::<i32>()
}
const TIMER_ID: usize = 0xF1;
const TIMER_MS: u32 = 300;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Btn {
    Mode,
    Width,
    Symbols,
    Settings,
}

/// 四个按钮，从左到右
const LAYOUT: [Btn; 4] = [Btn::Mode, Btn::Width, Btn::Symbols, Btn::Settings];

fn button_rects() -> [(Btn, RECT); 4] {
    let mut out = [(Btn::Mode, RECT::default()); 4];
    let mut x = pad();
    for (i, btn) in LAYOUT.iter().enumerate() {
        let w = crate::dpi::px(BTN_W[i]);
        let inset = btn_inset();
        out[i] = (
            *btn,
            RECT {
                left: x,
                top: inset,
                right: x + w,
                bottom: height() - inset,
            },
        );
        x += w;
    }
    out
}

static WND: AtomicIsize = AtomicIsize::new(0);

// ─────────────────────────────────────────────────────────────
// 悬停提示
// ─────────────────────────────────────────────────────────────
//
// 浮窗只有 30px 高、按钮上也就一个字（「中」「全」「符号」），新用户根本不知道
// 那个「全」是干什么的、更不知道键盘上怎么切。这里自己画一个小气泡，鼠标停在
// 按钮上就出来，写明「这个按钮是什么」+「快捷键是什么」。
//
// 不用系统 `tooltips_class32`：它要 `TOOLINFOW` + `TTM_ADDTOOL` 一整套，而字体、
// 圆角、配色全都跟候选窗对不上——一个输入法的浮窗边上冒出一个 Windows 98 风格的
// 黄条，很出戏。自己画就二十行，还能跟着换肤走。
const TIP_CLASS: PCWSTR = w!("FeiheStatusTip");
/// 提示窗与浮窗之间的空隙
const TIP_GAP_L: i32 = 4;
/// 提示窗内边距
const TIP_PAD_L: i32 = 8;

fn tip_gap() -> i32 {
    crate::dpi::px(TIP_GAP_L)
}
fn tip_pad() -> i32 {
    crate::dpi::px(TIP_PAD_L)
}

static TIP_WND: AtomicIsize = AtomicIsize::new(0);
static TIP_CLASS_REGISTERED: AtomicBool = AtomicBool::new(false);
/// 当前显示的是第几个按钮的提示。-2 = 没显示（-1 是 `HOVER` 的「不在任何按钮上」，
/// 两者分开，才能区分「鼠标在空隙上」和「提示本来就没开」）
static TIP_IDX: AtomicI32 = AtomicI32::new(-2);
static TIP_TEXT: Mutex<String> = Mutex::new(String::new());

/// 每个按钮的悬停说明。**快捷键要写在这里**——浮窗上没有第二处能放说明文字。
///
/// 只有「中 / 英」有键盘出口（单击 Shift）。全角 / 半角不加快捷键：`Ctrl+Shift+F`
/// 试过一版，但它只是个不常用的开关，占一个全局组合键不值，还容易和 IDE 抢键。
fn tip_for(btn: Btn) -> &'static str {
    match btn {
        Btn::Mode => "中 / 英切换（单击 Shift）",
        Btn::Width => "全角 / 半角（点一下切换）",
        Btn::Symbols => "符号软键盘",
        Btn::Settings => "设置",
    }
}
static CLASS_REGISTERED: AtomicBool = AtomicBool::new(false);
static VISIBLE: AtomicBool = AtomicBool::new(false);
static HOVER: AtomicI32 = AtomicI32::new(-1);
/// 创建浮窗的进程 id。比线程 id 稳：WinUI3/CoreText 宿主里 TIP 的激活线程
/// 不一定是拥有窗口的那个线程，比线程会永远判成「不是前台」而不显示。
static OWNER_PID: AtomicI32 = AtomicI32::new(0);
/// 最近一次已知的光标位置（跟随模式下用）
static CARET: Mutex<(i32, i32)> = Mutex::new((0, 0));
/// 拖动状态。`btn` 是**按下时**命中的按钮，抬起时要求同一个才算点击。
///
/// 为什么不能按抬起位置分发：四个按钮的矩形是连续的、中间没有间隙
/// （`button_rects_are_contiguous` 钉住了这一点），而拖动阈值是 3px。在
/// 「中/英」右缘按下、往右漂 2px 抬起，按下点还算是「中/英」，抬起点已经
/// 落到「全/半」上——中英没切、全半角被翻了。候选窗早就要求「按下/抬起
/// 同一项」（`candidate.rs` 的 pressed 判定），浮窗这条也一样。
#[derive(Clone, Copy)]
struct Drag {
    /// 按下点的屏幕坐标
    down: (i32, i32),
    /// 位移是否已超过阈值（进入拖动）
    moved: bool,
    /// 窗口原位
    origin: (i32, i32),
    /// 按下时命中的按钮下标（按在按钮之间的空隙里 = None）
    btn: Option<usize>,
}
static DRAG: Mutex<Option<Drag>> = Mutex::new(None);

fn hwnd() -> HWND {
    let raw = WND.load(Ordering::Acquire);
    HWND(raw as *mut core::ffi::c_void)
}

unsafe fn ensure_class() -> bool {
    if CLASS_REGISTERED.load(Ordering::Acquire) {
        return true;
    }
    let hinst = GetModuleHandleW(None).unwrap_or_default();
    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wndproc),
        hInstance: HINSTANCE(hinst.0),
        lpszClassName: CLASS_NAME,
        ..Default::default()
    };
    if RegisterClassW(&wc) == 0 {
        return false;
    }
    CLASS_REGISTERED.store(true, Ordering::Release);
    true
}

/// 默认位置：工作区右下角（避开任务栏）
unsafe fn default_pos() -> (i32, i32) {
    let mut rc = RECT::default();
    let ok = SystemParametersInfoW(
        SPI_GETWORKAREA,
        0,
        Some(&mut rc as *mut _ as *mut core::ffi::c_void),
        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
    )
    .is_ok();
    if !ok {
        rc.right = GetSystemMetrics(SM_CXSCREEN);
        rc.bottom = GetSystemMetrics(SM_CYSCREEN);
    }
    (rc.right - width() - crate::dpi::px(24), rc.bottom - height() - crate::dpi::px(24))
}

/// 矩形落在哪块显示器上，返回那块的工作区。拿不到返回 `None`。
unsafe fn monitor_work(rc: &RECT) -> Option<RECT> {
    let hmon = MonitorFromRect(rc, MONITOR_DEFAULTTONEAREST);
    if hmon.0.is_null() {
        return None;
    }
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    GetMonitorInfoW(hmon, &mut mi).as_bool().then_some(mi.rcWork)
}

/// 把 `(x, y)` 处一个 `w × h` 的窗口夹进**它所在的那块显示器**的工作区。
///
/// 为什么不能用 `SM_CXSCREEN/SM_CYSCREEN`：那是**主屏**尺寸。副屏上的窗口
/// （主屏右侧的副屏坐标 > 主屏宽；主屏左侧的副屏坐标是负的）会被生生钳回
/// 主屏——把浮窗拖到副屏，之后任何一次设置变更（`settings` 末尾无条件调
/// `statusbar::refresh()`）都让它跳回主屏；跟随模式下在副屏打字，浮窗离光标
/// 半个屏；主屏左侧的副屏更糟，坐标被钳到 0，直接消失。
///
/// 拿不到显示器信息才退回主屏尺寸——那条老路只在极端失败下走到。
unsafe fn clamp_to_monitor(x: i32, y: i32, w: i32, h: i32) -> (i32, i32) {
    let rc = RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    };
    let work = monitor_work(&rc).unwrap_or(RECT {
        left: 0,
        top: 0,
        right: GetSystemMetrics(SM_CXSCREEN),
        bottom: GetSystemMetrics(SM_CYSCREEN),
    });
    let x = x.clamp(work.left, (work.right - w).max(work.left));
    let y = y.clamp(work.top, (work.bottom - h).max(work.top));
    (x, y)
}

/// 把窗口摆到应该待的位置
unsafe fn place(hwnd: HWND, follow_caret: bool) {
    // 先刷新 DPI 再算坐标和尺寸：/ 都按当前 DPI 缩放，
    // 用旧的算出来会把窗口摆到错的位置（尤其是拖动到另一块屏之后）。
    {
        let (cx, cy) = *CARET.lock().unwrap();
        if follow_caret && (cx != 0 || cy != 0) {
            crate::dpi::update_for_point(cx, cy);
        } else {
            let cfg = config::get();
            if cfg.bar_x != 0 || cfg.bar_y != 0 {
                crate::dpi::update_for_point(cfg.bar_x, cfg.bar_y);
            } else {
                crate::dpi::update_for_system();
            }
        }
    }
    let (x, y) = if follow_caret {
        let (cx, cy) = *CARET.lock().unwrap();
        if cx == 0 && cy == 0 {
            default_pos()
        } else {
            (cx, cy + crate::dpi::px(24))
        }
    } else {
        let cfg = config::get();
        if cfg.bar_x == 0 && cfg.bar_y == 0 {
            default_pos()
        } else {
            (cfg.bar_x, cfg.bar_y)
        }
    };
    // 夹在屏幕内，避免改分辨率后跑到看不见的地方（按所在显示器夹，见函数注释）
    let (x, y) = clamp_to_monitor(x, y, width(), height());
    let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, width(), height(), SWP_NOACTIVATE);
}

unsafe fn create() -> Option<HWND> {
    if !ensure_class() {
        return None;
    }
    let hinst = GetModuleHandleW(None).unwrap_or_default();
    let hwnd = CreateWindowExW(
        WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
        CLASS_NAME,
        w!("飞鹤输入法"),
        WS_POPUP,
        0,
        0,
        width(),
        height(),
        None,
        None,
        Some(HINSTANCE(hinst.0)),
        None,
    )
    .unwrap_or_default();
    if hwnd.is_invalid() {
        return None;
    }
    OWNER_PID.store(GetCurrentProcessId() as i32, Ordering::Release);
    place(hwnd, config::get().bar_follow);
    SetTimer(Some(hwnd), TIMER_ID, TIMER_MS, None);
    Some(hwnd)
}

/// 显示状态浮窗（幂等）
pub fn show() {
    // 同 `candidate::show`：只在「不弹的原因」变化时记一行。
    static REFUSE: AtomicU8 = AtomicU8::new(255);

    // 不弹窗模式下一个窗口都不弹，理由同 `candidate::show`。
    if config::no_popup() {
        crate::dlog_change!(
            REFUSE,
            1,
            "statusbar::show 被 no_popup 拦下（scene={} fg={}）",
            crate::fullscreen::scene().key(),
            crate::diag::fg_class()
        );
        hide();
        return;
    }
    // 全屏场景下浮窗单独再拦一道。**这和候选窗不是一回事**：候选窗是「按了键才
    // 出现、选完就消失」，全屏游戏里必须有；浮窗是常驻的，全屏时会一直压在游戏
    // 画面上挡视线。所以这里判的是 `scene` 而不是 `no_popup`——全屏游戏里
    // `no_popup` 已经是 false（候选要弹）。
    if crate::fullscreen::scene().hides_statusbar() {
        crate::dlog_change!(
            REFUSE,
            3,
            "statusbar::show 被全屏拦下（scene={}）",
            crate::fullscreen::scene().key()
        );
        hide();
        return;
    }
    REFUSE.store(0, Ordering::Relaxed);
    let raw = WND.load(Ordering::Acquire);
    let h = if raw != 0 {
        HWND(raw as *mut core::ffi::c_void)
    } else {
        match unsafe { create() } {
            Some(h) => {
                WND.store(h.0 as isize, Ordering::Release);
                h
            }
            None => {
                crate::dlog_change!(REFUSE, 2, "statusbar::show：窗口创建失败");
                return;
            }
        }
    };
    unsafe {
        let _ = ShowWindow(h, SW_SHOWNA);
        let _ = InvalidateRect(Some(h), None, true);
    }
    VISIBLE.store(true, Ordering::Release);
}

/// 隐藏状态浮窗（不弹窗模式、UILess 宿主、以及设置窗切换开关时用）
pub fn hide() {
    let h = hwnd();
    if h.is_invalid() {
        return;
    }
    unsafe {
        let _ = ShowWindow(h, SW_HIDE);
    }
    VISIBLE.store(false, Ordering::Release);
}

/// 反激活/卸载时销毁
pub fn destroy() {
    let raw = WND.swap(0, Ordering::AcqRel);
    VISIBLE.store(false, Ordering::Release);
    if raw == 0 {
        return;
    }
    unsafe {
        let h = HWND(raw as *mut core::ffi::c_void);
        let _ = KillTimer(Some(h), TIMER_ID);
        let _ = DestroyWindow(h);
        // 提示窗是独立窗口，得单独销毁，否则进程退出时留一个孤儿 topmost
        TIP_IDX.store(-2, Ordering::Release);
        let tip = HWND(TIP_WND.swap(0, Ordering::AcqRel) as *mut core::ffi::c_void);
        if !tip.is_invalid() {
            let _ = DestroyWindow(tip);
        }
    }
}

/// TIP 报告光标位置：跟随模式下浮窗贴到光标下方
pub fn set_caret(x: i32, y: i32) {
    if let Ok(mut g) = CARET.lock() {
        *g = (x, y);
    }
    if config::get().bar_follow {
        let h = hwnd();
        if !h.is_invalid() {
            unsafe { place(h, true) };
        }
    }
}

/// 浮窗当前屏幕位置（关闭「跟随光标」时用它把当前位置固定下来）
pub fn current_pos() -> Option<(i32, i32)> {
    let h = hwnd();
    if h.is_invalid() {
        return None;
    }
    let mut rc = RECT::default();
    unsafe { GetWindowRect(h, &mut rc) }.ok()?;
    Some((rc.left, rc.top))
}

/// 配置变了（换肤、改跟随）后刷新外观与位置
pub fn refresh() {
    let h = hwnd();
    if h.is_invalid() {
        return;
    }
    unsafe {
        place(h, config::get().bar_follow);
        let _ = InvalidateRect(Some(h), None, true);
    }
}

/// 只重绘、不重排位置。中英状态之类的变化走这里——重排会把正在拖动的浮窗拽回去。
pub fn repaint() {
    let h = hwnd();
    if h.is_invalid() {
        return;
    }
    unsafe {
        let _ = InvalidateRect(Some(h), None, true);
    }
}

// ─────────────────────────────────────────────────────────────
// 绘制
// ─────────────────────────────────────────────────────────────

unsafe fn paint(h: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(h, &mut ps);
    if hdc.is_invalid() {
        return;
    }
    let mut rc = RECT::default();
    let _ = GetClientRect(h, &mut rc);

    let p = theme::current();
    font::fill(hdc, &rc, p.bg);
    font::frame(hdc, &rc, p.border);

    let f = font::ui();
    SelectObject(hdc, f);
    SetBkMode(hdc, TRANSPARENT);

    let chinese = insert::chinese();
    let fullwidth = config::get().fullwidth;
    let hover = HOVER.load(Ordering::Acquire);

    for (i, (btn, r)) in button_rects().iter().enumerate() {
        if hover == i as i32 {
            font::fill(hdc, r, p.hover);
        }
        let (label, color) = match btn {
            Btn::Mode => {
                // 繁体模式没有单独的按钮（浮窗要瘦），用这个按钮的字兼作提示：
                // 英文态显示「英」，中文态按字形显示「中」/「繁」。宽度不变。
                if !chinese {
                    ("英", p.dim)
                } else if config::get().traditional {
                    ("繁", p.on)
                } else {
                    ("中", p.on)
                }
            }
            Btn::Width => {
                if fullwidth {
                    ("全", p.on)
                } else {
                    ("半", p.dim)
                }
            }
            Btn::Symbols => ("符号", p.text),
            Btn::Settings => ("设置", p.text),
        };
        font::text_center(hdc, r, label, color);
    }

    let _ = EndPaint(h, &ps);
}

// ─────────────────────────────────────────────────────────────
// 悬停提示窗
// ─────────────────────────────────────────────────────────────

unsafe fn ensure_tip_class() -> bool {
    if TIP_CLASS_REGISTERED.load(Ordering::Acquire) {
        return true;
    }
    let hinst = GetModuleHandleW(None).unwrap_or_default();
    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(tip_wndproc),
        hInstance: HINSTANCE(hinst.0),
        lpszClassName: TIP_CLASS,
        ..Default::default()
    };
    if RegisterClassW(&wc) == 0 {
        return false;
    }
    TIP_CLASS_REGISTERED.store(true, Ordering::Release);
    true
}

unsafe fn tip_paint(h: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(h, &mut ps);
    if hdc.is_invalid() {
        return;
    }
    let mut rc = RECT::default();
    let _ = GetClientRect(h, &mut rc);

    let p = theme::current();
    font::fill(hdc, &rc, p.bg);
    font::frame(hdc, &rc, p.border);

    let text = TIP_TEXT.lock().map(|t| t.clone()).unwrap_or_default();
    if !text.is_empty() {
        SelectObject(hdc, font::ui());
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, p.text);
        let mut w: Vec<u16> = text.encode_utf16().collect();
        DrawTextW(hdc, &mut w, &mut rc, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
    }
    let _ = EndPaint(h, &ps);
}

unsafe extern "system" fn tip_wndproc(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            tip_paint(h);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        // 提示窗绝不抢焦点，也不吃鼠标（`WS_EX_TRANSPARENT` 已经让点击穿过去）
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        _ => DefWindowProcW(h, msg, w, l),
    }
}

/// 显示第 `idx` 个按钮的提示。`owner` 是状态浮窗（用来定位）。
///
/// 幂等：同一个按钮重复调用直接返回，**不重排窗口**。定时器每 300ms 轮询一次，
/// 每次都 SetWindowPos 的话提示会一直在原地抖。
unsafe fn tip_show(owner: HWND, idx: usize) {
    if TIP_IDX.load(Ordering::Acquire) == idx as i32 {
        return;
    }
    let rects = button_rects();
    let Some((btn, rect)) = rects.get(idx).copied() else {
        return;
    };
    if !ensure_tip_class() {
        return;
    }

    let mut tip = HWND(TIP_WND.load(Ordering::Acquire) as *mut core::ffi::c_void);
    if tip.is_invalid() {
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        tip = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
            TIP_CLASS,
            w!(""),
            WS_POPUP,
            0,
            0,
            10,
            10,
            None,
            None,
            Some(HINSTANCE(hinst.0)),
            None,
        )
        .unwrap_or_default();
        if tip.is_invalid() {
            return;
        }
        TIP_WND.store(tip.0 as isize, Ordering::Release);
    }

    let text = tip_for(btn);
    if let Ok(mut t) = TIP_TEXT.lock() {
        t.clear();
        t.push_str(text);
    }

    // 量一下文字要多大。用提示窗自己的 DC，字体和候选窗一致。
    let hdc = GetDC(Some(tip));
    let mut calc = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    if !hdc.is_invalid() {
        let old = SelectObject(hdc, font::ui());
        let mut w: Vec<u16> = text.encode_utf16().collect();
        DrawTextW(hdc, &mut w, &mut calc, DT_CALCRECT | DT_SINGLELINE);
        SelectObject(hdc, old);
        let _ = ReleaseDC(Some(tip), hdc);
    }
    let tw = calc.right + tip_pad() * 2;
    let th = calc.bottom + tip_pad() * 2;

    let mut o = RECT::default();
    let _ = GetWindowRect(owner, &mut o);
    // 默认贴在浮窗下面，与按钮左边缘对齐；底部放不下就翻到上面。
    // 上下沿都按**浮窗所在显示器**的工作区算（拿不到才退主屏）：副屏在主屏
    // 下方时，用主屏底部沿判断会认为「还放得下」，气泡直接跑到屏幕外。
    let work = monitor_work(&o).unwrap_or(RECT {
        left: 0,
        top: 0,
        right: GetSystemMetrics(SM_CXSCREEN),
        bottom: GetSystemMetrics(SM_CYSCREEN),
    });
    let mut x = o.left + rect.left;
    let mut y = o.bottom + tip_gap();
    if y + th > work.bottom {
        y = (o.top - th - tip_gap()).max(work.top);
    }
    x = x.clamp(work.left, (work.right - tw).max(work.left));

    let _ = SetWindowPos(
        tip,
        Some(HWND_TOPMOST),
        x,
        y,
        tw,
        th,
        SWP_NOACTIVATE | SWP_SHOWWINDOW,
    );
    let _ = InvalidateRect(Some(tip), None, true);
    TIP_IDX.store(idx as i32, Ordering::Release);
}

unsafe fn tip_hide() {
    // `-2` 已经是「没显示」，别每次都往里塞一次 ShowWindow
    if TIP_IDX.swap(-2, Ordering::AcqRel) == -2 {
        return;
    }
    let tip = HWND(TIP_WND.load(Ordering::Acquire) as *mut core::ffi::c_void);
    if !tip.is_invalid() {
        let _ = ShowWindow(tip, SW_HIDE);
    }
}

/// 轮询鼠标：停在哪个按钮上就出哪个提示，离开就收。
///
/// 为什么不用 `WM_MOUSELEAVE`：`TrackMouseEvent` 在 windows-rs 0.62 里没有导出
/// （和 `SS_NOTIFY` 一个待遇），而浮窗本来就有 300ms 定时器——顺手在这里判一次
/// 更省事，也顺便把「鼠标甩出去、按钮还亮着」的高亮残留一起修掉。
unsafe fn tip_follow_mouse(h: HWND) {
    if !VISIBLE.load(Ordering::Acquire) {
        TIP_IDX.store(-2, Ordering::Release);
        let tip = HWND(TIP_WND.load(Ordering::Acquire) as *mut core::ffi::c_void);
        if !tip.is_invalid() {
            let _ = ShowWindow(tip, SW_HIDE);
        }
        return;
    }
    let mut pt = POINT::default();
    if GetCursorPos(&mut pt).is_err() {
        return;
    }
    let mut cr = pt;
    let _ = ScreenToClient(h, &mut cr);
    // 拖动中不出提示：那会儿用户是在挪窗口，弹个气泡反而挡手
    let dragging = DRAG
        .lock()
        .ok()
        .and_then(|g| g.map(|s| s.moved))
        .unwrap_or(false);
    let idx = if dragging { None } else { hit(cr.x, cr.y) };

    // 高亮也跟着对齐（鼠标快速移出窗口时 mousemove 不会再来）
    let want = idx.map(|i| i as i32).unwrap_or(-1);
    if HOVER.swap(want, Ordering::AcqRel) != want {
        let _ = InvalidateRect(Some(h), None, true);
    }

    match idx {
        Some(i) => tip_show(h, i),
        None => tip_hide(),
    }
}

// ─────────────────────────────────────────────────────────────
// 交互
// ─────────────────────────────────────────────────────────────

fn hit(x: i32, y: i32) -> Option<usize> {
    button_rects()
        .iter()
        .position(|(_, r)| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
}

/// 抬起时算不算「点了某个按钮」：**按下和抬起必须命中同一个**。
///
/// 抽成纯函数是为了单测——真正的判定在窗口过程里，那边构造不出鼠标事件。
fn click_idx(down: Option<usize>, up: Option<usize>) -> Option<usize> {
    down.filter(|&b| Some(b) == up)
}

unsafe fn on_click(idx: usize) {
    // 气泡先收。**不能等鼠标移开**：点「设置」会弹出模态设置窗并抢前台，
    // 而光标通常还停在按钮上，`tip_show` 的幂等早退会让气泡一直浮在设置窗
    // 上面；点「符号」弹出的软键盘正好开在气泡所在的位置，会压在键盘上沿。
    tip_hide();
    match LAYOUT[idx] {
        Btn::Mode => {
            let next = !insert::chinese();
            if insert::set_chinese(next) {
            }
        }
        Btn::Width => {
            config::update(|c| c.fullwidth = !c.fullwidth);
        }
        Btn::Symbols => {
            let h = hwnd();
            let mut rc = RECT::default();
            let _ = GetWindowRect(h, &mut rc);
            softkbd::toggle(rc.left, rc.bottom + 4);
        }
        Btn::Settings => settings::open(),
    }
    let h = hwnd();
    if !h.is_invalid() {
        let _ = InvalidateRect(Some(h), None, true);
    }
}

unsafe extern "system" fn wndproc(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(h);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        // 拖动到缩放不同的屏幕上：系统先告诉新 DPI，再让我们按新尺寸重排。
        // 不处理的话窗口还按旧 DPI 的宽高，字会溢出按钮、高亮块错位。
        WM_DPICHANGED => {
            crate::dpi::update_for_window(h);
            place(h, config::get().bar_follow);
            let _ = InvalidateRect(Some(h), None, true);
            LRESULT(0)
        }
        // 绝不抢焦点
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),

        WM_LBUTTONDOWN => {
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let mut wr = RECT::default();
            let _ = GetWindowRect(h, &mut wr);
            // 按下时命中的按钮记下来，抬起时必须是同一个才算点击（见 `Drag`）
            let mut cr = POINT { x: pt.x, y: pt.y };
            let _ = ScreenToClient(h, &mut cr);
            if let Ok(mut g) = DRAG.lock() {
                *g = Some(Drag {
                    down: (pt.x, pt.y),
                    moved: false,
                    origin: (wr.left, wr.top),
                    btn: hit(cr.x, cr.y),
                });
            }
            SetCapture(h);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let mut dragging = false;
            if let Ok(mut g) = DRAG.lock() {
                if let Some(d) = *g {
                    let dx = pt.x - d.down.0;
                    let dy = pt.y - d.down.1;
                    if !d.moved && (dx.abs() > 3 || dy.abs() > 3) {
                        if let Some(slot) = g.as_mut() {
                            slot.moved = true;
                        }
                    }
                    dragging = g.map(|s| s.moved).unwrap_or(false);
                    if dragging {
                        let _ = SetWindowPos(
                            h,
                            Some(HWND_TOPMOST),
                            d.origin.0 + dx,
                            d.origin.1 + dy,
                            0,
                            0,
                            SWP_NOSIZE | SWP_NOACTIVATE,
                        );
                    }
                }
            }
            if !dragging {
                // 悬停高亮
                let mut cr = POINT { x: pt.x, y: pt.y };
                let _ = ScreenToClient(h, &mut cr);
                let idx = hit(cr.x, cr.y).map(|i| i as i32).unwrap_or(-1);
                if HOVER.swap(idx, Ordering::AcqRel) != idx {
                    let _ = InvalidateRect(Some(h), None, true);
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            // **先取状态，再 ReleaseCapture。** 顺序不能反：`ReleaseCapture()`
            // 会**同步**给本窗口发一条 `WM_CAPTURECHANGED`，而那个分支要把
            // `DRAG` 清掉（那是给「捕获被别的窗口抢走」用的）。先释放就会把
            // 这次点击自己抹掉——表现为「点按钮完全没反应」，且不报任何错。
            // 候选窗早在 WM_LBUTTONUP 的注释里记着这个坑。
            let state = DRAG.lock().ok().and_then(|mut g| g.take());
            let _ = ReleaseCapture();
            match state {
                Some(d) if d.moved => {
                    // 拖动结束：记住位置（跟随模式下不记，位置由光标决定）
                    if !config::get().bar_follow {
                        let mut wr = RECT::default();
                        let _ = GetWindowRect(h, &mut wr);
                        config::update(|c| {
                            c.bar_x = wr.left;
                            c.bar_y = wr.top;
                        });
                    }
                }
                Some(d) => {
                    // 点击：**按下和抬起必须是同一个按钮**（见 `Drag` 的注释）。
                    // 按钮矩形零间隙 + 3px 拖动阈值，按抬起位置分发会让 2px 的
                    // 手抖变成「点了隔壁按钮」。
                    let mut cr = POINT { x: pt.x, y: pt.y };
                    let _ = ScreenToClient(h, &mut cr);
                    if let Some(idx) = click_idx(d.btn, hit(cr.x, cr.y)) {
                        on_click(idx);
                    }
                }
                None => {}
            }
            LRESULT(0)
        }
        WM_TIMER if w.0 == TIMER_ID => {
            // 只在自己线程是前台线程时显示，避免多个宿主进程各飘一条
            let fg = GetForegroundWindow();
            let mut pid: u32 = 0;
            let mine = if fg.is_invalid() {
                false
            } else {
                GetWindowThreadProcessId(fg, Some(&mut pid));
                pid == OWNER_PID.load(Ordering::Acquire) as u32
            };
            let shown = VISIBLE.load(Ordering::Acquire);
            if mine && !shown {
                // **必须走 `show()`，不能直接 `ShowWindow`。** 直接弹会绕过
                // `no_popup` 和全屏两道闸：全屏游戏里 `show()` 刚按场景把自己
                // 收起，300ms 后定时器又把它放出来，于是一直挂在游戏画面上；
                // 用户在设置里选了「一律不弹」也会被定时器悄悄撤销。
                // `show()` 幂等且含全部闸门（该收起时它自己会 `hide()`），
                // 顺带让定时器成为场景变化的重新评估点。
                show();
            } else if !mine && shown {
                let _ = ShowWindow(h, SW_HIDE);
                VISIBLE.store(false, Ordering::Release);
            }
            tip_follow_mouse(h);
            LRESULT(0)
        }
        // 拖动中丢了鼠标捕获（本线程弹模态框、别的窗口 `SetCapture`），
        // `WM_LBUTTONUP` 就不会再来。不清的话 `DRAG` 停在「已在拖动」，
        // 之后鼠标划过窗口就拖着它走——用户没按任何键，浮窗跟着跑。
        // 候选窗对同样的场景有对应处理（清 `pressed`）。
        WM_CAPTURECHANGED | WM_CANCELMODE => {
            if let Ok(mut g) = DRAG.lock() {
                *g = None;
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(h, msg, w, l),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个按钮都得有悬停说明——浮窗上没有第二处能写「这个按钮是干什么的」。
    #[test]
    fn every_button_has_a_tip() {
        for (i, b) in LAYOUT.iter().enumerate() {
            assert!(!tip_for(*b).is_empty(), "第 {i} 个按钮没有悬停说明");
        }
        assert!(
            tip_for(Btn::Mode).contains("Shift"),
            "中/英的提示里必须写 Shift"
        );
    }

    /// **按下和抬起必须命中同一个按钮**，否则不算点击。
    ///
    /// 现场：按钮矩形零间隙（有用例钉住），拖动阈值 3px。在「中/英」右缘
    /// 按下、向右抖 2px 抬起，按抬起位置分发会命中「全/半」——中英没切、
    /// 全半角被翻了。抬起点和按下点不一致时正确的行为是「什么都没发生」。
    #[test]
    fn click_needs_the_same_button_on_press_and_release() {
        assert_eq!(click_idx(Some(0), Some(0)), Some(0));
        assert_eq!(click_idx(Some(1), Some(1)), Some(1));
        assert_eq!(
            click_idx(Some(0), Some(1)),
            None,
            "按下「中/英」、抬起「全/半」不许触发"
        );
        assert_eq!(click_idx(Some(1), Some(0)), None);
        // 按在按钮之间的空隙里（None）不该激活任何按钮
        assert_eq!(click_idx(None, Some(0)), None);
        assert_eq!(click_idx(Some(0), None), None);
        assert_eq!(click_idx(None, None), None);
    }

    /// 全角 / 半角**不能**再挂快捷键。`Ctrl+Shift+F` 曾经存在过一版，撤掉是因为
    /// 这开关不常用，占一个全局组合键不值（还会和 IDE 的全局搜索抢）。
    /// 提示里出现 `Ctrl+` 就说明它被加回来了。
    #[test]
    fn width_button_has_no_shortcut() {
        assert!(
            !tip_for(Btn::Width).contains("Ctrl"),
            "全/半不该再挂快捷键：{}",
            tip_for(Btn::Width)
        );
    }

    /// 按钮不重叠、按顺序铺满浮窗宽度——提示是按 `button_rects()` 定位的，
    /// 这里错位的话提示气泡会飘到隔壁按钮下面。
    #[test]
    fn button_rects_are_contiguous() {
        let rects = button_rects();
        assert_eq!(rects.len(), LAYOUT.len());
        assert_eq!(rects[0].1.left, pad());
        for w in rects.windows(2) {
            assert_eq!(w[0].1.right, w[1].1.left, "按钮之间有缝或有重叠");
        }
        assert_eq!(rects[rects.len() - 1].1.right, width() - pad());
    }
}
