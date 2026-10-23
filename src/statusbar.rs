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

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, Ordering};
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

const PAD: i32 = 6;
const HEIGHT: i32 = 30;
const TIMER_ID: usize = 0xF1;
const TIMER_MS: u32 = 300;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Btn {
    Mode,
    Width,
    Symbols,
    Settings,
}

/// (按钮, 宽度)
const LAYOUT: [(Btn, i32); 4] = [
    (Btn::Mode, 34),
    (Btn::Width, 34),
    (Btn::Symbols, 44),
    (Btn::Settings, 44),
];

const WIDTH: i32 = PAD * 2 + 34 + 34 + 44 + 44;

fn button_rects() -> [(Btn, RECT); 4] {
    let mut out = [(Btn::Mode, RECT::default()); 4];
    let mut x = PAD;
    for (i, (btn, w)) in LAYOUT.iter().enumerate() {
        out[i] = (
            *btn,
            RECT {
                left: x,
                top: 3,
                right: x + w,
                bottom: HEIGHT - 3,
            },
        );
        x += w;
    }
    out
}

static WND: AtomicIsize = AtomicIsize::new(0);
static CLASS_REGISTERED: AtomicBool = AtomicBool::new(false);
static VISIBLE: AtomicBool = AtomicBool::new(false);
static HOVER: AtomicI32 = AtomicI32::new(-1);
/// 创建浮窗的进程 id。比线程 id 稳：WinUI3/CoreText 宿主里 TIP 的激活线程
/// 不一定是拥有窗口的那个线程，比线程会永远判成「不是前台」而不显示。
static OWNER_PID: AtomicI32 = AtomicI32::new(0);
/// 最近一次已知的光标位置（跟随模式下用）
static CARET: Mutex<(i32, i32)> = Mutex::new((0, 0));
/// 拖动状态：(按下点屏幕坐标, 是否已在拖动, 窗口原位)
static DRAG: Mutex<Option<((i32, i32), bool, (i32, i32))>> = Mutex::new(None);

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
    (rc.right - WIDTH - 24, rc.bottom - HEIGHT - 24)
}

/// 把窗口摆到应该待的位置
unsafe fn place(hwnd: HWND, follow_caret: bool) {
    let (mut x, mut y) = if follow_caret {
        let (cx, cy) = *CARET.lock().unwrap();
        if cx == 0 && cy == 0 {
            default_pos()
        } else {
            (cx, cy + 24)
        }
    } else {
        let cfg = config::get();
        if cfg.bar_x == 0 && cfg.bar_y == 0 {
            default_pos()
        } else {
            (cfg.bar_x, cfg.bar_y)
        }
    };
    // 夹在屏幕内，避免改分辨率后跑到看不见的地方
    let sw = GetSystemMetrics(SM_CXSCREEN);
    let sh = GetSystemMetrics(SM_CYSCREEN);
    x = x.clamp(0, (sw - WIDTH).max(0));
    y = y.clamp(0, (sh - HEIGHT).max(0));
    let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, WIDTH, HEIGHT, SWP_NOACTIVATE);
}

unsafe fn create() -> Option<HWND> {
    if !ensure_class() {
        crate::trace!("bar: RegisterClass failed");
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
        WIDTH,
        HEIGHT,
        None,
        None,
        Some(HINSTANCE(hinst.0)),
        None,
    )
    .unwrap_or_default();
    if hwnd.is_invalid() {
        crate::trace!("bar: CreateWindowEx failed");
        return None;
    }
    OWNER_PID.store(GetCurrentProcessId() as i32, Ordering::Release);
    place(hwnd, config::get().bar_follow);
    SetTimer(Some(hwnd), TIMER_ID, TIMER_MS, None);
    crate::trace!("bar: created");
    Some(hwnd)
}

/// 显示状态浮窗（幂等）
pub fn show() {
    let raw = WND.load(Ordering::Acquire);
    let h = if raw != 0 {
        HWND(raw as *mut core::ffi::c_void)
    } else {
        match unsafe { create() } {
            Some(h) => {
                WND.store(h.0 as isize, Ordering::Release);
                h
            }
            None => return,
        }
    };
    unsafe {
        let _ = ShowWindow(h, SW_SHOWNA);
        let _ = InvalidateRect(Some(h), None, true);
    }
    VISIBLE.store(true, Ordering::Release);
}

#[allow(dead_code)]
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
// 交互
// ─────────────────────────────────────────────────────────────

fn hit(x: i32, y: i32) -> Option<usize> {
    button_rects()
        .iter()
        .position(|(_, r)| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
}

unsafe fn on_click(idx: usize) {
    match LAYOUT[idx].0 {
        Btn::Mode => {
            let next = !insert::chinese();
            if insert::set_chinese(next) {
                crate::trace!("bar: chinese -> {next}");
            }
        }
        Btn::Width => {
            config::update(|c| c.fullwidth = !c.fullwidth);
            crate::trace!("bar: fullwidth -> {}", config::get().fullwidth);
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
        // 绝不抢焦点
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),

        WM_LBUTTONDOWN => {
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let mut wr = RECT::default();
            let _ = GetWindowRect(h, &mut wr);
            if let Ok(mut g) = DRAG.lock() {
                *g = Some(((pt.x, pt.y), false, (wr.left, wr.top)));
            }
            SetCapture(h);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let mut dragging = false;
            if let Ok(mut g) = DRAG.lock() {
                if let Some((down, moved, origin)) = *g {
                    let dx = pt.x - down.0;
                    let dy = pt.y - down.1;
                    if !moved && (dx.abs() > 3 || dy.abs() > 3) {
                        if let Some(slot) = g.as_mut() {
                            slot.1 = true;
                        }
                    }
                    dragging = g.map(|s| s.1).unwrap_or(false);
                    if dragging {
                        let _ = SetWindowPos(
                            h,
                            Some(HWND_TOPMOST),
                            origin.0 + dx,
                            origin.1 + dy,
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
            let _ = ReleaseCapture();
            let state = DRAG.lock().ok().and_then(|mut g| g.take());
            match state {
                Some((_, true, _)) => {
                    // 拖动结束：记住位置（跟随模式下不记，位置由光标决定）
                    if !config::get().bar_follow {
                        let mut wr = RECT::default();
                        let _ = GetWindowRect(h, &mut wr);
                        config::update(|c| {
                            c.bar_x = wr.left;
                            c.bar_y = wr.top;
                        });
                        crate::trace!("bar: moved to ({},{})", wr.left, wr.top);
                    }
                }
                Some((_, false, _)) => {
                    let mut cr = POINT { x: pt.x, y: pt.y };
                    let _ = ScreenToClient(h, &mut cr);
                    if let Some(idx) = hit(cr.x, cr.y) {
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
                let _ = ShowWindow(h, SW_SHOWNA);
                let _ = InvalidateRect(Some(h), None, true);
                VISIBLE.store(true, Ordering::Release);
            } else if !mine && shown {
                let _ = ShowWindow(h, SW_HIDE);
                VISIBLE.store(false, Ordering::Release);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(h, msg, w, l),
    }
}
