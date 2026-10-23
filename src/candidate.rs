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
//! **右键**走另一条回调：`Ctrl`+数字 在很多多页签程序里被浏览器/编辑器抢走
//! （`Ctrl+1..9` 切页签），所以置顶必须有一个只用鼠标的入口。右键不做菜单，
//! 直接切换置顶状态——再右击一次就取消，和 `Ctrl`+数字 的语义一致。
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
/// 候选词和它后面那串读音小字之间的缝
const HINT_GAP: i32 = 5;

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

/// 左键：选中该候选（上屏 / 确认分词段）
static ON_CLICK: Mutex<Option<Handler>> = Mutex::new(None);
/// 右键：切换该候选的置顶状态（不提交）
static ON_RIGHT: Mutex<Option<Handler>> = Mutex::new(None);

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

/// 注册「第 N 个候选被右键」的回调。语义同 `set_click_handler`。
pub fn set_right_click_handler(id: usize, make: impl FnOnce() -> ClickFn) {
    let mut g = match ON_RIGHT.lock() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    if g.as_ref().is_some_and(|h| h.id == id) {
        return;
    }
    *g = Some(Handler { id, f: make() });
}

pub fn clear_right_click_handler() {
    let mut g = match ON_RIGHT.lock() {
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

/// 同 `invoke_click`，走右键那个槽位
fn invoke_right(index: usize) {
    let f = match ON_RIGHT.lock() {
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

/// 这一项要不要在词后面标读音（`chóng qìng`）。
///
/// 只有 `hint` 表里的「易错词」才有——全库 4.5 万条里 753 条。全标会把候选窗
/// 撑成两倍宽，而绝大多数词（`你好`、`我们`）读音毫无歧义，标了纯属噪音。
fn hint_of(word: &str) -> Option<&'static str> {
    if !config::reading() {
        return None;
    }
    crate::hint::toned(word)
}

/// 量一项的宽度：主文字 + 可选读音小字。
///
/// 两个字号必须**分别**量：小字比主字矮 1px、窄一截，拿主字的宽度去摆小字会
/// 让相邻候选之间忽宽忽窄。绘制和命中判定都调这里，才不会出现「看着在这、
/// 点不到」的偏差。
unsafe fn measure(hdc: HDC, text: &str, hint: Option<&str>) -> (i32, i32) {
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut size = SIZE::default();
    let _ = GetTextExtentPoint32W(hdc, &wide, &mut size);

    let hw = match hint {
        None => 0,
        Some(h) => {
            let old = SelectObject(hdc, crate::font::small());
            let hwide: Vec<u16> = h.encode_utf16().collect();
            let mut hs = SIZE::default();
            let _ = GetTextExtentPoint32W(hdc, &hwide, &mut hs);
            SelectObject(hdc, old);
            hs.cx
        }
    };
    (size.cx, hw)
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

/// 从鼠标消息的 `lParam` 取出客户区 x 坐标（低 16 位，带符号）。
///
/// **不要用 `GetCursorPos`**：那是「处理消息时」的鼠标位置，不是「消息发生那一刻」
/// 的位置。用户点得快一点、或者点击后顺手移开，就会算到相邻的候选上去。鼠标消息的
/// `lParam` 本身就是事件发生时的客户区坐标，Windows 已经替我们记好了。
fn client_x(l: LPARAM) -> i32 {
    (l.0 as u32 & 0xFFFF) as u16 as i16 as i32
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
                let mut st = match STATE.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                let idx = hit(&st, client_x(l));
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
            // 顺序不能反：`ReleaseCapture()` 会**同步**给本窗口发 WM_CAPTURECHANGED，
            // 而那个分支要把 `pressed` 清掉（那是给「捕获被别人抢走」用的）。
            // 先释放就会把这次点击自己抹掉——表现为「点候选完全没反应」，
            // 而且不会报任何错。所以先把命中判定做完再释放。
            let target = {
                let mut st = match STATE.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                let up = hit(&st, client_x(l));
                let pressed = st.pressed.take();
                // 按下和抬起必须落在同一项，否则算误触
                if pressed.is_some() && pressed == up {
                    up
                } else {
                    None
                }
            };
            let _ = ReleaseCapture();
            // 锁已释放，这里才回调
            if let Some(i) = target {
                invoke_click(i);
            }
            LRESULT(0)
        }
        // 右键 = 置顶开关。不需要按下/抬起配对：菜单式的误触风险不存在，
        // 而「按下就动作」在多页签程序里更跟手（右键菜单一次都没弹过）。
        WM_RBUTTONUP => {
            let idx = {
                let st = match STATE.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                hit(&st, client_x(l))
            };
            if let Some(i) = idx {
                invoke_right(i);
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
    paint_content(hdc, &rc);

    let _ = EndPaint(hwnd, &ps);
}

/// 真正的绘制。抽成函数是为了能在测试里画到内存 DC 上截图——`paint` 绑死
/// `BeginPaint`，没有窗口就没法验证布局，而「小字提示把相邻候选挤歪」这类问题
/// 恰恰只有画出来才看得见。
unsafe fn paint_content(hdc: HDC, rc: &RECT) {
    let p = crate::theme::current();
    crate::font::fill(hdc, rc, p.bg);
    crate::font::frame(hdc, rc, p.border);

    SelectObject(hdc, crate::font::ui());
    SetBkMode(hdc, TRANSPARENT);

    let st = STATE.lock().unwrap();
    let items = page_slice(&st);
    if items.is_empty() {
        return;
    }

    let mut x = PAD;
    for (i, word) in items.iter().enumerate() {
        let text = label(i, word);
        let wide: Vec<u16> = text.encode_utf16().collect();
        let hint = hint_of(word);

        let mut size = SIZE::default();
        let _ = GetTextExtentPoint32W(hdc, &wide, &mut size);
        let (_, hw) = measure(hdc, &text, hint);
        let total = size.cx + if hint.is_some() { HINT_GAP + hw } else { 0 };

        // st.selected 是页内序号，不是全局序号
        if i == st.selected {
            let hi = RECT {
                left: x - 4,
                top: 3,
                right: x + total + 4,
                bottom: rc.bottom - 3,
            };
            crate::font::fill(hdc, &hi, p.accent);
            SetTextColor(hdc, p.accent_text);
        } else {
            SetTextColor(hdc, p.text);
        }

        let ty = (rc.bottom - size.cy) / 2;
        let _ = TextOutW(hdc, x, ty, &wide);

        // 读音紧跟词后，小一号、灰一档：它是注解，不能和候选本身抢注意力
        if let Some(h) = hint {
            let hwide: Vec<u16> = h.encode_utf16().collect();
            let old = SelectObject(hdc, crate::font::small());
            SetTextColor(hdc, if i == st.selected { p.accent_text } else { p.dim });
            let mut hs = SIZE::default();
            let _ = GetTextExtentPoint32W(hdc, &hwide, &mut hs);
            // 比主文字低 2px，视觉上挂在词的下沿，不跟序号抢同一行
            let _ = TextOutW(hdc, x + size.cx + HINT_GAP, ty + 2, &hwide);
            SelectObject(hdc, old);
            // 小字画完要把颜色恢复给下一个候选用（SetTextColor 不随字体还原）
            SetTextColor(hdc, if i == st.selected { p.accent_text } else { p.text });
        }

        x += total + GAP;
    }
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
        let text = label(i, word);
        let (mw, hw) = measure(hdc, &text, hint_of(word));
        let total = mw + if hw > 0 { HINT_GAP + hw } else { 0 };
        spans.push((x - 4, x + total + 4));
        x += total + GAP;
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
    // 该收起时一个 popup 都不许弹（「始终不弹」或自动检测到全屏，见 `config::no_popup`）。
    //
    // 拦在这里而不是拦在 `textservice::show_candidates`，是因为这个函数是**所有**
    // 显示路径的唯一出口；拦在上层的话，将来新加一个调用点就会漏。
    // 顺带把可能已经挂着的窗口收掉——用户可能刚切到全屏，或者刚在设置里改了档。
    //
    // 注意这是**我们这边**的判断；宿主驱动的 UILess 走 `uielement`，不经过这里。
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
    clear_right_click_handler();
    let raw = WND.swap(0, Ordering::AcqRel);
    if raw == 0 {
        return;
    }
    unsafe {
        let hwnd = HWND(raw as *mut core::ffi::c_void);
        let _ = DestroyWindow(hwnd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `lParam` 的低 16 位是客户区 x，按 i16 解释（窗口贴屏幕左边缘时 x 可能为负）。
    #[test]
    fn client_x_sign_extends() {
        assert_eq!(client_x(LPARAM(0x0005_0003)), 3);
        assert_eq!(client_x(LPARAM(0x0005_0000)), 0);
        // 高 16 位是 y，不能污染 x
        assert_eq!(client_x(LPARAM(0x03FF_0012)), 0x12);
        // x = 0xFFFF 应解释成 -1，而不是 65535
        assert_eq!(client_x(LPARAM(0x0000_FFFF)), -1);
    }

    /// 端到端验证「点候选」这条路径：真建窗口、真发鼠标消息、真走回调。
    ///
    /// 这段代码以前一行测试都没有，于是「点了没反应」只能靠人工点——而人工点分不清
    /// 是「消息没到窗口」「命中算错」还是「回调没注册」。这里三件事一起钉住。
    ///
    /// 用 `SendMessageW` 而不是 `SendInput`：消息直接进 `wndproc`，不需要光标真的在
    /// 那个位置，CI 里也能跑（前提是有桌面——没有窗口站的纯服务环境会失败）。
    #[test]
    fn click_and_right_click_reach_handlers() {
        // STATE / 回调槽都是进程级静态，两个用例并行会互相踩
        static LOCK: Mutex<()> = Mutex::new(());
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());

        crate::config::set_no_popup_for_test(false);
        show(&["甲", "乙", "丙"], 0, 0, 200, 200);

        let hwnd = unsafe { FindWindowW(CLASS_NAME, w!("")) }.unwrap_or_default();
        assert!(!hwnd.is_invalid(), "候选窗没建出来");

        // 取第一项的中心：命中区的几何就是布局算出来那份
        let (x1, x2) = {
            STATE
                .lock()
                .unwrap()
                .spans
                .first()
                .copied()
                .expect("spans 为空")
        };
        let px = (x1 + x2) / 2;
        let lp = |x: i32| Some(LPARAM((((15i32) << 16) | (x & 0xFFFF)) as isize));

        static LEFT: AtomicIsize = AtomicIsize::new(-1);
        static RIGHT: AtomicIsize = AtomicIsize::new(-1);
        LEFT.store(-1, Ordering::Release);
        RIGHT.store(-1, Ordering::Release);
        set_click_handler(7, || {
            Arc::new(|i: usize| LEFT.store(i as isize, Ordering::Release))
        });
        set_right_click_handler(7, || {
            Arc::new(|i: usize| RIGHT.store(i as isize, Ordering::Release))
        });

        unsafe {
            SendMessageW(hwnd, WM_LBUTTONDOWN, Some(WPARAM(0)), lp(px));
            SendMessageW(hwnd, WM_LBUTTONUP, Some(WPARAM(0)), lp(px));
            assert_eq!(LEFT.load(Ordering::Acquire), 0, "左键应命中第 0 项");

            SendMessageW(hwnd, WM_RBUTTONUP, Some(WPARAM(0)), lp(px));
            assert_eq!(RIGHT.load(Ordering::Acquire), 0, "右键应命中第 0 项");

            // 按下和抬起不在同一项 = 误触，不该回调
            SendMessageW(hwnd, WM_LBUTTONDOWN, Some(WPARAM(0)), lp(px));
            SendMessageW(hwnd, WM_LBUTTONUP, Some(WPARAM(0)), lp(x2 + 500));
            assert_eq!(LEFT.load(Ordering::Acquire), 0, "抬起跑偏后不该再有回调");

            // 窗口外的点（x 为负）也不该命中
            SendMessageW(hwnd, WM_RBUTTONUP, Some(WPARAM(0)), Some(LPARAM(-1)));
            assert_eq!(RIGHT.load(Ordering::Acquire), 0, "负坐标不该命中");
        }

        hide();
        clear_click_handler();
        clear_right_click_handler();
    }

    /// 命中区是左闭右开；边界落在右端点时属于下一项，这是「不留 1px 重叠」的约定。
    #[test]
    fn hit_uses_half_open_intervals() {
        let mut st = CandState {
            items: vec!["甲".into(), "乙".into()],
            selected: 0,
            page: 0,
            x: 0,
            y: 0,
            spans: vec![(6, 30), (30, 54)],
            pressed: None,
        };
        assert_eq!(hit(&st, 5), None, "左边留白不该命中");
        assert_eq!(hit(&st, 6), Some(0));
        assert_eq!(hit(&st, 29), Some(0));
        assert_eq!(hit(&st, 30), Some(1));
        assert_eq!(hit(&st, 54), None, "右端点属于不存在的下一项");
        st.spans.clear();
        assert_eq!(hit(&st, 10), None);
    }

    /// 把一页候选画到内存位图上存 BMP，人眼看布局。
    ///
    /// **不进 CI**：它不断言任何像素，只是把「小字提示、高亮框、候选间距」画出来。
    /// 读音提示这种东西的毛病（挤在一起、被裁掉、灰得看不见）没有一条能用断言表达。
    ///
    ///     cargo test --lib snapshot -- --ignored --nocapture
    ///
    /// 产出 `target/candidate_snapshot.bmp`。
    #[test]
    #[ignore = "人工看布局用，不做断言"]
    fn snapshot_hint_layout() {
        static LOCK: Mutex<()> = Mutex::new(());
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let words = ["重庆", "弹窗", "你好", "银行", "水雷屯", "我们"];
        {
            let mut st = STATE.lock().unwrap();
            st.items = words.iter().map(|s| s.to_string()).collect();
            st.selected = 1;
            st.page = 0;
        }
        let items: Vec<String> = words.iter().map(|s| s.to_string()).collect();
        let (w, spans) = unsafe { layout(&items) };
        assert!(w > 100 && spans.len() == words.len());

        unsafe {
            let screen = GetDC(None);
            let mem = CreateCompatibleDC(Some(screen));
            let bmp = CreateCompatibleBitmap(screen, w, HEIGHT);
            let old = SelectObject(mem, bmp.into());
            let rc = RECT {
                left: 0,
                top: 0,
                right: w,
                bottom: HEIGHT,
            };
            paint_content(mem, &rc);

            let stride = ((w * 3 + 3) / 4 * 4) as usize;
            let mut bi = BITMAPINFO::default();
            bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bi.bmiHeader.biWidth = w;
            bi.bmiHeader.biHeight = -HEIGHT; // 负数 = 自上而下，省得再翻转
            bi.bmiHeader.biPlanes = 1;
            bi.bmiHeader.biBitCount = 24;
            bi.bmiHeader.biCompression = BI_RGB.0;
            let mut pix = vec![0u8; stride * HEIGHT as usize];
            GetDIBits(
                mem,
                bmp,
                0,
                HEIGHT as u32,
                Some(pix.as_mut_ptr() as *mut core::ffi::c_void),
                &mut bi,
                DIB_RGB_COLORS,
            );

            SelectObject(mem, old);
            let _ = DeleteObject(HGDIOBJ(bmp.0));
            let _ = DeleteDC(mem);

            // 24 位 BMP：14 字节文件头 + 40 字节信息头
            let mut out = Vec::with_capacity(54 + pix.len());
            let size = (54 + pix.len()) as u32;
            out.extend_from_slice(b"BM");
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&[0, 0, 0, 0]);
            out.extend_from_slice(&54u32.to_le_bytes());
            out.extend_from_slice(&40u32.to_le_bytes());
            out.extend_from_slice(&w.to_le_bytes());
            out.extend_from_slice(&(-HEIGHT).to_le_bytes());
            out.extend_from_slice(&1u16.to_le_bytes());
            out.extend_from_slice(&24u16.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&(pix.len() as u32).to_le_bytes());
            out.extend_from_slice(&[0u8; 16]);
            out.extend_from_slice(&pix);

            let path = concat!(env!("CARGO_MANIFEST_DIR"), "/target/candidate_snapshot.bmp");
            std::fs::write(path, &out).expect("写截图失败");
            println!("截图：{path}（{w}x{HEIGHT}）");
        }
    }
}
