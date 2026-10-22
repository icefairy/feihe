//! 特殊符号软键盘
//!
//! 9 列 × 5 行的自绘网格，点一下把符号插到当前文档光标处。
//!
//! 插入走 `insert::insert_text`（同步 edit session），不合成按键——很多符号根本
//! 没有对应键位，靠 SendInput 打不出来。
//!
//! 面板不抢焦点，所以收不到「点到别处」的通知。关闭时机靠两处：
//! 再点一次状态栏的「符号」按钮，或者在应用里敲了任意键（TIP 会调 `hide`）。

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, Ordering};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::font;
use crate::insert;
use crate::theme;

const CLASS_NAME: PCWSTR = w!("FeiheSoftKeyboard");

const COLS: usize = 9;
const CELL_W: i32 = 36;
const CELL_H: i32 = 34;
const PAD: i32 = 6;

const SYMBOLS: [&str; COLS * 5] = [
    "，", "。", "、", "；", "：", "？", "！", "…", "～",
    "“", "”", "‘", "’", "（", "）", "《", "》", "【",
    "＋", "－", "×", "÷", "＝", "≠", "≈", "≤", "≥",
    "％", "‰", "℃", "°", "±", "∞", "√", "∑", "∏",
    "★", "☆", "○", "●", "△", "▲", "→", "←", "↓",
];

const ROWS: usize = 5;
const WIDTH: i32 = PAD * 2 + CELL_W * COLS as i32;
const HEIGHT: i32 = PAD * 2 + CELL_H * ROWS as i32;

static WND: AtomicIsize = AtomicIsize::new(0);
static CLASS_REGISTERED: AtomicBool = AtomicBool::new(false);
static HOVER: AtomicI32 = AtomicI32::new(-1);

fn hwnd() -> HWND {
    let raw = WND.load(Ordering::Acquire);
    HWND(raw as *mut core::ffi::c_void)
}

fn cell_rect(i: usize) -> RECT {
    let r = (i / COLS) as i32;
    let c = (i % COLS) as i32;
    RECT {
        left: PAD + c * CELL_W,
        top: PAD + r * CELL_H,
        right: PAD + (c + 1) * CELL_W,
        bottom: PAD + (r + 1) * CELL_H,
    }
}

fn hit(x: i32, y: i32) -> Option<usize> {
    (0..SYMBOLS.len()).find(|&i| {
        let r = cell_rect(i);
        x >= r.left && x < r.right && y >= r.top && y < r.bottom
    })
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

unsafe fn ensure_window() -> Option<HWND> {
    let raw = WND.load(Ordering::Acquire);
    if raw != 0 {
        return Some(HWND(raw as *mut core::ffi::c_void));
    }
    if !ensure_class() {
        crate::trace!("kbd: RegisterClass failed");
        return None;
    }
    let hinst = GetModuleHandleW(None).unwrap_or_default();
    let h = CreateWindowExW(
        WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
        CLASS_NAME,
        w!("特殊符号"),
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
    if h.is_invalid() {
        crate::trace!("kbd: CreateWindowEx failed");
        return None;
    }
    WND.store(h.0 as isize, Ordering::Release);
    Some(h)
}

/// 显示在 (x, y)（屏幕坐标，通常是状态浮窗左下角）
pub fn show_at(x: i32, y: i32) {
    unsafe {
        let Some(h) = ensure_window() else { return };
        let sw = GetSystemMetrics(SM_CXSCREEN);
        let sh = GetSystemMetrics(SM_CYSCREEN);
        let mut px = x;
        let mut py = y;
        if px + WIDTH > sw {
            px = (sw - WIDTH).max(0);
        }
        if py + HEIGHT > sh {
            // 下方放不下就翻到上方
            py = (py - HEIGHT - 34).max(0);
        }
        let _ = SetWindowPos(
            h,
            Some(HWND_TOPMOST),
            px,
            py,
            WIDTH,
            HEIGHT,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        let _ = InvalidateRect(Some(h), None, true);
    }
}

pub fn toggle(x: i32, y: i32) {
    let h = hwnd();
    if h.is_invalid() {
        show_at(x, y);
        return;
    }
    let visible = unsafe { IsWindowVisible(h).as_bool() };
    if visible {
        hide();
    } else {
        show_at(x, y);
    }
}

pub fn hide() {
    let h = hwnd();
    if h.is_invalid() {
        return;
    }
    unsafe {
        let _ = ShowWindow(h, SW_HIDE);
    }
}

pub fn destroy() {
    let raw = WND.swap(0, Ordering::AcqRel);
    HOVER.store(-1, Ordering::Release);
    if raw == 0 {
        return;
    }
    unsafe {
        let _ = DestroyWindow(HWND(raw as *mut core::ffi::c_void));
    }
}

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

    SelectObject(hdc, font::ui());
    SetBkMode(hdc, TRANSPARENT);

    let hover = HOVER.load(Ordering::Acquire);
    for (i, sym) in SYMBOLS.iter().enumerate() {
        let r = cell_rect(i);
        if hover == i as i32 {
            font::fill(hdc, &r, p.accent);
            font::text_center(hdc, &r, sym, p.accent_text);
        } else {
            font::text_center(hdc, &r, sym, p.text);
        }
    }

    let _ = EndPaint(h, &ps);
}

unsafe extern "system" fn wndproc(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(h);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_MOUSEMOVE => {
            let x = (l.0 & 0xFFFF) as i16 as i32;
            let y = ((l.0 >> 16) & 0xFFFF) as i16 as i32;
            let idx = hit(x, y).map(|i| i as i32).unwrap_or(-1);
            if HOVER.swap(idx, Ordering::AcqRel) != idx {
                let _ = InvalidateRect(Some(h), None, true);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let x = (l.0 & 0xFFFF) as i16 as i32;
            let y = ((l.0 >> 16) & 0xFFFF) as i16 as i32;
            if let Some(idx) = hit(x, y) {
                let sym = SYMBOLS[idx];
                let ok = insert::insert_text(sym);
                crate::trace!("kbd: insert {sym:?} -> {ok}");
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(h, msg, w, l),
    }
}
