//! 候选窗：自绘 popup 窗口
//!
//! 没用 `ITfUIElement` / `ITfCandidateListUIElement`——那套要额外实现
//! 四五个 COM 接口加一套 UIElementSink 生命周期，代码量翻倍。这里直接开一个
//! `WS_POPUP | WS_EX_NOACTIVATE | WS_EX_TOPMOST` 的窗口用 GDI 画，效果一样，
//! 而且不会被宿主应用的窗口层级盖住。
//!
//! v1 只支持键盘选词（空格 / 数字 / 方向键），鼠标点击暂未接——
//! 窗口会吞掉点击但不会把事件透给下层应用，避免误移光标导致组合被打断。

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Mutex;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

const CLASS_NAME: PCWSTR = w!("FeiheCandidateWindow");

const PAD: i32 = 10;
const GAP: i32 = 16;
const HEIGHT: i32 = 30;

/// 一页最多几个候选（数字键 1-9）
pub const PAGE_SIZE: usize = 9;

struct CandState {
    items: Vec<String>,
    selected: usize,
    page: usize,
    x: i32,
    y: i32,
}

static STATE: Mutex<CandState> = Mutex::new(CandState {
    items: Vec::new(),
    selected: 0,
    page: 0,
    x: 0,
    y: 0,
});

static WND: AtomicIsize = AtomicIsize::new(0);
static CLASS_REGISTERED: AtomicBool = AtomicBool::new(false);

/// 候选显示文本：`1.词`。
///
/// 简繁转换放在这里而不是查词时做：候选表里存的永远是词库原文，切换字形不用
/// 重建任何东西，也不会因为转换而影响「选中项对齐到哪个词」的判断。
fn label(index_in_page: usize, word: &str) -> String {
    format!("{}.{}", index_in_page + 1, crate::zhconv::to_traditional(word))
}

fn page_slice(st: &CandState) -> &[String] {
    let start = st.page * PAGE_SIZE;
    if start >= st.items.len() {
        return &[];
    }
    let end = (start + PAGE_SIZE).min(st.items.len());
    &st.items[start..end]
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

unsafe fn ensure_window() -> HWND {
    let raw = WND.load(Ordering::Acquire);
    if raw != 0 {
        return HWND(raw as *mut core::ffi::c_void);
    }
    let hinst = GetModuleHandleW(None).unwrap_or_default();
    let hwnd = CreateWindowExW(
        WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
        CLASS_NAME,
        w!(""),
        WS_POPUP,
        0,
        0,
        10,
        HEIGHT,
        None,
        None,
        Some(HINSTANCE(hinst.0)),
        None,
    )
    .unwrap_or_default();
    WND.store(hwnd.0 as isize, Ordering::Release);
    hwnd
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        // 自绘背景，避免闪烁
        WM_ERASEBKGND => LRESULT(1),
        // 点候选窗不抢焦点，否则应用会失焦、组合态被断掉
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}

unsafe fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    if hdc.is_invalid() {
        return;
    }

    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);

    let p = crate::theme::current();
    crate::font::fill(hdc, &rc, p.bg);
    crate::font::frame(hdc, &rc, p.border);

    SelectObject(hdc, crate::font::ui());
    SetBkMode(hdc, TRANSPARENT);

    let st = STATE.lock().unwrap();
    let items = page_slice(&st);
    if items.is_empty() {
        let _ = EndPaint(hwnd, &ps);
        return;
    }

    let mut x = PAD;
    for (i, word) in items.iter().enumerate() {
        let text = label(i, word);
        let wide: Vec<u16> = text.encode_utf16().collect();

        let mut size = SIZE::default();
        let _ = GetTextExtentPoint32W(hdc, &wide, &mut size);

        // st.selected 是页内序号，不是全局序号
        if i == st.selected {
            let hi = RECT {
                left: x - 4,
                top: 3,
                right: x + size.cx + 4,
                bottom: rc.bottom - 3,
            };
            crate::font::fill(hdc, &hi, p.accent);
            SetTextColor(hdc, p.accent_text);
        } else {
            SetTextColor(hdc, p.text);
        }

        let _ = TextOutW(hdc, x, (rc.bottom - size.cy) / 2, &wide);
        x += size.cx + GAP;
    }

    let _ = EndPaint(hwnd, &ps);
}

/// 换肤后重绘（窗口不存在时什么也不做）
pub fn refresh_theme() {
    let raw = WND.load(Ordering::Acquire);
    if raw == 0 {
        return;
    }
    unsafe {
        let _ = InvalidateRect(Some(HWND(raw as *mut core::ffi::c_void)), None, true);
    }
}

/// 计算窗口宽度
unsafe fn measure(items: &[String]) -> i32 {
    if items.is_empty() {
        return PAD * 2;
    }
    let hdc = GetDC(None);
    if hdc.is_invalid() {
        return 320;
    }
    let font = crate::font::ui();
    SelectObject(hdc, font);

    let mut total = PAD * 2;
    for (i, word) in items.iter().enumerate() {
        let wide: Vec<u16> = label(i, word).encode_utf16().collect();
        let mut size = SIZE::default();
        let _ = GetTextExtentPoint32W(hdc, &wide, &mut size);
        total += size.cx;
    }
    total += GAP * (items.len() as i32 - 1);
    let _ = ReleaseDC(None, hdc);
    total.max(80)
}

/// 显示候选窗。`x` / `y` 是光标左下角的屏幕坐标。
pub fn show(items: &[&str], selected: usize, page: usize, x: i32, y: i32) {
    if items.is_empty() {
        hide();
        return;
    }
    unsafe {
        if !ensure_class() {
            crate::trace!("cand: RegisterClass failed");
            return;
        }
    }

    {
        let mut st = STATE.lock().unwrap();
        st.items.clear();
        st.items.extend(items.iter().map(|s| s.to_string()));
        st.selected = selected;
        st.page = page;
        st.x = x;
        st.y = y;
    }

    unsafe {
        let hwnd = ensure_window();
        if hwnd.is_invalid() {
            crate::trace!("cand: CreateWindowEx failed");
            return;
        }
        let st = STATE.lock().unwrap();
        let page_items: Vec<String> = page_slice(&st).to_vec();
        drop(st);

        let w = measure(&page_items);

        // 贴光标下方 6px，并夹在屏幕内
        let sw = GetSystemMetrics(SM_CXSCREEN);
        let sh = GetSystemMetrics(SM_CYSCREEN);
        let mut px = x;
        let mut py = y + 6;
        if px + w > sw {
            px = (sw - w).max(0);
        }
        if py + HEIGHT > sh {
            // 下方放不下就翻到光标上方
            py = (y - HEIGHT - 6).max(0);
        }

        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            px,
            py,
            w,
            HEIGHT,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        let _ = InvalidateRect(Some(hwnd), None, true);
        crate::trace!(
            "cand: show at ({px},{py}) {w}x{HEIGHT} caret=({x},{y}) screen={sw}x{sh} items={} page={page} sel={selected}",
            page_items.len()
        );
    }
}

pub fn hide() {
    let raw = WND.load(Ordering::Acquire);
    if raw == 0 {
        return;
    }
    unsafe {
        let hwnd = HWND(raw as *mut core::ffi::c_void);
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

/// 卸载/反激活时销毁窗口，避免 DLL 被卸载后窗口过程指针失效
pub fn destroy() {
    let raw = WND.swap(0, Ordering::AcqRel);
    if raw == 0 {
        return;
    }
    unsafe {
        let hwnd = HWND(raw as *mut core::ffi::c_void);
        let _ = DestroyWindow(hwnd);
    }
}
