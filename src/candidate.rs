//! 候选窗：自绘 popup 窗口
//!
//! 这里**不用** `ITfUIElement` / `ITfCandidateListUIElement`——那套要走宿主的
//! `ITfUIElementSink` 协商，宿主不接就没有候选。桌面程序全是「输入法自己画」，
//! 所以主路径就开一个 `WS_POPUP | WS_EX_NOACTIVATE | WS_EX_TOPMOST` 的窗口用 GDI 画，
//! 效果一样，而且不会被宿主应用的窗口层级盖住。
//!
//! 那套接口实现放在 `uielement` 模块，只服务一种情况：宿主声明自己是 UI-less 线程
//! （独占全屏游戏），此时我们的窗口根本画不上屏，候选必须交给宿主自己画。
//!
//! ## 鼠标点击
//!
//! 窗口是 `NOACTIVATE` 的，点它不会抢焦点、也不会打断组合态。点击通过
//! `set_click_handler` 注册的回调路由回文本服务——本模块不认识 `textservice`，
//! 只负责「第几个候选被点了」。
//!
//! 判定用「按下 + 抬起落在同一项」：按下时记下命中项并 `SetCapture`，
//! 抬起时比对，中途移开就取消。这样和普通按钮的手感一致，也避免误触。
//!
//! 命中区**必须和绘制用同一份几何**（`layout()`），否则会出现「看着点到了、
//! 实际没中」这种最难查的偏差。

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
// SetCapture / ReleaseCapture 归在 Input::KeyboardAndMouse，不在 WindowsAndMessaging
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config;

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
    /// 每一项的横向命中区（相对客户区），由 `layout()` 算出
    spans: Vec<(i32, i32)>,
    /// 按下时命中的项，抬起时比对
    pressed: Option<usize>,
}

static STATE: Mutex<CandState> = Mutex::new(CandState {
    items: Vec::new(),
    selected: 0,
    page: 0,
    x: 0,
    y: 0,
    spans: Vec::new(),
    pressed: None,
});

static WND: AtomicIsize = AtomicIsize::new(0);
static CLASS_REGISTERED: AtomicBool = AtomicBool::new(false);

type ClickFn = Arc<dyn Fn(usize) + Send + Sync>;

struct Handler {
    /// 注册者的身份（文本服务对象的地址），用来判断要不要重建闭包
    id: usize,
    f: ClickFn,
}

static ON_CLICK: Mutex<Option<Handler>> = Mutex::new(None);

/// 注册「第 N 个候选被点击」的回调。
///
/// `id` 是注册者身份：同一个对象重复注册直接返回，不用每敲一个键就重建闭包。
/// `make` 只在确实需要换人时才调用。
pub fn set_click_handler(id: usize, make: impl FnOnce() -> ClickFn) {
    let mut g = match ON_CLICK.lock() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    if g.as_ref().is_some_and(|h| h.id == id) {
        return;
    }
    *g = Some(Handler { id, f: make() });
}

/// 撤销回调（对象销毁时调，避免留着一个指向已释放对象的闭包）
pub fn clear_click_handler() {
    let mut g = match ON_CLICK.lock() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    *g = None;
}

/// 调用回调。**不能持锁调用**：回调会走 edit session，而 edit session 里可能
/// 又调 `show()` → `set_click_handler()`，持锁就成了自己等自己。
fn invoke_click(index: usize) {
    let f = match ON_CLICK.lock() {
        Ok(g) => g.as_ref().map(|h| h.f.clone()),
        Err(e) => e.into_inner().as_ref().map(|h| h.f.clone()),
    };
    if let Some(f) = f {
        f(index);
    }
}

/// 候选显示文本：`1.词`。
///
/// 简繁转换放在这里而不是查词时做：候选表里存的永远是词库原文，切换字形不用
/// 重建任何东西，也不会因为转换而影响「选中项对齐到哪个词」的判断。
fn label(index_in_page: usize, word: &str) -> String {
    format!("{}.{}", index_in_page + 1, crate::zhconv::apply(word))
}

fn page_slice(st: &CandState) -> &[String] {
    let start = st.page * PAGE_SIZE;
    if start >= st.items.len() {
        return &[];
    }
    let end = (start + PAGE_SIZE).min(st.items.len());
    &st.items[start..end]
}

fn hit(st: &CandState, x: i32) -> Option<usize> {
    st.spans.iter().position(|(l, r)| x >= *l && x < *r)
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

        WM_LBUTTONDOWN => {
            let idx = {
                let mut pt = POINT::default();
                let _ = GetCursorPos(&mut pt);
                let _ = ScreenToClient(hwnd, &mut pt);
                let mut st = match STATE.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                let idx = hit(&st, pt.x);
                st.pressed = idx;
                idx
            };
            // 捕获鼠标：在窗口外抬起也能收到 WM_LBUTTONUP，用来取消这次点击
            if idx.is_some() {
                SetCapture(hwnd);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let _ = ReleaseCapture();
            let target = {
                let mut pt = POINT::default();
                let _ = GetCursorPos(&mut pt);
                let _ = ScreenToClient(hwnd, &mut pt);
                let mut st = match STATE.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                let up = hit(&st, pt.x);
                let pressed = st.pressed.take();
                // 按下和抬起必须落在同一项，否则算误触
                if pressed.is_some() && pressed == up {
                    up
                } else {
                    None
                }
            };
            // 锁已释放，这里才回调
            if let Some(i) = target {
                invoke_click(i);
            }
            LRESULT(0)
        }
        // 鼠标捕获被别人抢走（比如弹了个系统菜单）→ 这次点击作废
        WM_CAPTURECHANGED => {
            if let Ok(mut st) = STATE.lock() {
                st.pressed = None;
            }
            LRESULT(0)
        }
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

/// 计算窗口宽度和每一项的横向命中区。
///
/// 绘制（`paint`）和命中判定（`hit`）都依赖这里的几何：paint 按同样的
/// `size.cx + GAP` 递增，高亮框左右各放宽 4px，所以命中区也用 ±4。
unsafe fn layout(items: &[String]) -> (i32, Vec<(i32, i32)>) {
    if items.is_empty() {
        return (PAD * 2, Vec::new());
    }
    let hdc = GetDC(None);
    if hdc.is_invalid() {
        return (320, Vec::new());
    }
    SelectObject(hdc, crate::font::ui());

    let mut x = PAD;
    let mut spans = Vec::with_capacity(items.len());
    for (i, word) in items.iter().enumerate() {
        let wide: Vec<u16> = label(i, word).encode_utf16().collect();
        let mut size = SIZE::default();
        let _ = GetTextExtentPoint32W(hdc, &wide, &mut size);
        spans.push((x - 4, x + size.cx + 4));
        x += size.cx + GAP;
    }
    let _ = ReleaseDC(None, hdc);
    ((x - GAP + PAD).max(80), spans)
}

/// 显示候选窗。`x` / `y` 是光标左下角的屏幕坐标。
pub fn show(items: &[&str], selected: usize, page: usize, x: i32, y: i32) {
    if items.is_empty() {
        hide();
        return;
    }
    // 不弹窗模式：一个 popup 都不许弹。
    //
    // 拦在这里而不是拦在 `textservice::show_candidates`，是因为这个函数是**所有**
    // 显示路径的唯一出口；拦在上层的话，将来新加一个调用点就会漏。
    // 顺带把可能已经挂着的窗口收掉——用户是刚在设置里打开的开关。
    //
    // 注意这是**用户**开的开关；宿主驱动的 UILess 走 `uielement`，不经过这里。
    if config::no_popup() {
        hide();
        return;
    }
    unsafe {
        if !ensure_class() {
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
            return;
        }
        let st = STATE.lock().unwrap();
        let page_items: Vec<String> = page_slice(&st).to_vec();
        drop(st);

        let (w, spans) = layout(&page_items);
        if let Ok(mut st) = STATE.lock() {
            st.spans = spans;
            st.pressed = None;
        }

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
    // 窗口藏起来时可能还有一次没走完的按下，清掉，免得下次显示时误判
    if let Ok(mut st) = STATE.lock() {
        st.pressed = None;
        st.spans.clear();
    }
}

/// 卸载/反激活时销毁窗口，避免 DLL 被卸载后窗口过程指针失效
pub fn destroy() {
    clear_click_handler();
    let raw = WND.swap(0, Ordering::AcqRel);
    if raw == 0 {
        return;
    }
    unsafe {
        let hwnd = HWND(raw as *mut core::ffi::c_void);
        let _ = DestroyWindow(hwnd);
    }
}
