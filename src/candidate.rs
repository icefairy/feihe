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
//! **`Ctrl`+左键**走第三条：降权（把这个词压到候选最后，但不删）。左键是选词、
//! 右键是置顶，都占了，而降权又值得有一个鼠标入口——见 `demote` 的模块注释。
//!
//! 命中区**必须和绘制用同一份几何**（`layout()`），否则会出现「看着点到了、
//! 实际没中」这种最难查的偏差。

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, Ordering};
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

/// 鼠标消息 `wParam` 里的「Ctrl 正被按住」标志。
///
/// windows-rs 0.62 没导出 `MK_CONTROL`（和 `SS_NOTIFY` 一样，只有头文件里有），
/// 写字面值。这个值从 Win32 定义至今没变过。
const MK_CONTROL: usize = 0x0008;

/// 下面这些是 **100% 缩放（96 DPI）下的逻辑像素**。真机上要过一遍
/// [`crate::dpi::px`] 换成当前显示器的物理像素——否则 150% 缩放的笔记本上，
/// 候选窗只有别人的一半大（详见 `src/dpi.rs` 的模块注释）。
const PAD_L: i32 = 10;
const GAP_L: i32 = 16;
/// 一行的行高。单行时窗口高 `row_h() + 2`（= 30，和老版本一模一样），
/// 多行时就是 `rows * row_h() + 2`。
const ROW_H_L: i32 = 28;
/// 候选词和它后面那串读音小字之间的缝
const HINT_GAP_L: i32 = 5;

/// 留白、候选间距、行高、读音缝——全部按当前 DPI 放大。
///
/// 抽成函数而不是 `static`：DPI 会在运行中变（窗口拖到另一块屏、用户改缩放），
/// 缓存成常量就永远停在第一次的值上。
fn pad() -> i32 {
    crate::dpi::px(PAD_L)
}
fn gap() -> i32 {
    crate::dpi::px(GAP_L)
}
fn row_h() -> i32 {
    crate::dpi::px(ROW_H_L)
}
fn hint_gap() -> i32 {
    crate::dpi::px(HINT_GAP_L)
}

/// 选中高亮框在候选文字左右各放宽多少。命中区用同一个数——
/// 画得比命中区宽，用户会点在高亮上却没选中。
fn hit_pad() -> i32 {
    crate::dpi::px(4)
}

/// 单个候选（含后面的读音小字）的**宽度上限**。
///
/// 8 字词是本工程的常态上限（「依法追究刑事责任」），13px 雅黑下一个 8 字词约 110px，
/// 加上序号和读音小字接近 150px；9 个这样的候选横排就是 1400px 往上，比很多
/// 笔记本的整块屏幕还宽。超宽时窗口会被夹到屏幕左边，右边那一截直接
/// 看不见——**看得见但点不到**比不显示更糟。
///
/// 所以这里先按 240px 一刀切：超过就截断加省略号，把「打不下」这件事
/// 变得可见。
const MAX_ITEM_W_L: i32 = 240;

/// 单个候选的宽度上限（物理像素）。
///
/// 上限本身也要跟着 DPI 涨：96 DPI 下 240px 能放 8 个中文字，不留神的话
/// 在 200% 屏上还是 240 **物理** 像素，只够放 4 个字，明明是宽屏却截断。
fn max_item_w() -> i32 {
    crate::dpi::px(MAX_ITEM_W_L)
}

/// 窗口与屏幕左右边缘之间至少留出的距离。
///
/// 贴边会盖住滚动条，而且看起来像「窗口卡在屏幕外了」。左右各留 8px 之后，
/// 即使窗口宽度顶到上限，也还能看出这是一块浮在桌面上的窗。
const SCREEN_MARGIN_L: i32 = 8;

/// 窗口允许的最大宽度：屏幕宽减去两侧留白，且不小于一个字的宽度。
///
/// 抽成函数是为了让 layout 和 show 用同一个数——两处各算一次的话，
/// 迟早会出现「布局按 800 排、显示按 900 夹」这种半截候选。
fn max_window_w() -> i32 {
    unsafe { (GetSystemMetrics(SM_CXSCREEN) - 2 * crate::dpi::px(SCREEN_MARGIN_L)).max(crate::dpi::px(160)) }
}

/// 一页最多几个候选（数字键 1-9）
pub const PAGE_SIZE: usize = 9;

/// 窗口高度：`rows` 行，上下各 1px 给边框
fn height_of(rows: usize) -> i32 {
    rows as i32 * row_h() + crate::dpi::px(2)
}

/// 第 `page` 页的那一段候选
fn page_slice(items: &[String], page: usize) -> &[String] {
    let start = page * PAGE_SIZE;
    if start >= items.len() {
        return &[];
    }
    let end = (start + PAGE_SIZE).min(items.len());
    &items[start..end]
}

struct CandState {
    items: Vec<String>,
    /// 活动页内的选中序号（数字键选的就是**活动页**）
    selected: usize,
    /// 活动页。数字键选的是这一页；`=` 往下挪一页（视觉上就是「当前行下移一行」）
    page: usize,
    /// 顶部可见页。多行时 `top = page - (rows - 1)`，即活动页永远贴着最后一行，
    /// 再往后翻就是整窗上滚一行——这就是「第 6 行出现、第 1 行消失」。
    top: usize,
    /// 当前显示几行（1 = 老的单行）
    rows: usize,
    x: i32,
    y: i32,
    /// 每一行里每一项的横向命中区（相对客户区），由 `layout()` 算出
    spans: Vec<Vec<(i32, i32)>>,
    /// 每一行里每一项**实际画出来的字**（超宽时是截断过的，见 [max_item_w]）。
    ///
    /// 必须存下来而不是绘制时重算：重算要再量一遍字宽，而绘制是每次重绘都走的
    /// 热路径；更要紧的是「量出来多少宽」和「画了什么」得是同一份数据，
    /// 否则会出现截断后的字和命中区对不上。
    texts: Vec<Vec<String>>,
    /// 每一行里每一项要画的读音小字（超宽被砍掉时为 None）。
    ///
    /// 和 [texts] 一样必须存：读音是跟着「这串字画多宽」走的，重算一次就可能
    /// 因为字体度量变化而和命中区错开半像素。
    hints: Vec<Vec<Option<&'static str>>>,
    /// 按下时命中的项（全表下标），抬起时比对
    pressed: Option<usize>,
}

static STATE: Mutex<CandState> = Mutex::new(CandState {
    items: Vec::new(),
    selected: 0,
    page: 0,
    top: 0,
    rows: 1,
    x: 0,
    y: 0,
    spans: Vec::new(),
    texts: Vec::new(),
    hints: Vec::new(),
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
/// `Ctrl`+左键：切换该候选的降权状态（不提交）
static ON_DEMOTE: Mutex<Option<Handler>> = Mutex::new(None);

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

/// 注册「`Ctrl`+左键点了第 N 个候选」的回调。语义同 `set_click_handler`。
///
/// 为什么不是右键：右键已经是置顶。为什么不是又开一个全局快捷键：降权是低频动作，
/// 为它占一个全局组合键会和别的软件抢键（项目里刚因为同样的理由去掉过全角切换的
/// 快捷键）。`Ctrl`+左键是鼠标手势，不占任何键位。
pub fn set_demote_click_handler(id: usize, make: impl FnOnce() -> ClickFn) {
    let mut g = match ON_DEMOTE.lock() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    if g.as_ref().is_some_and(|h| h.id == id) {
        return;
    }
    *g = Some(Handler { id, f: make() });
}

pub fn clear_demote_click_handler() {
    let mut g = match ON_DEMOTE.lock() {
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

/// 同 `invoke_click`，走 `Ctrl`+左键那个槽位
fn invoke_demote(index: usize) {
    let f = match ON_DEMOTE.lock() {
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

/// 截断后的尾巴。用单个省略号而不是 `...`：中文全角字体里
/// 三个点占三个字宽，而省略号只占一个，在 240px 的预算里那 20px 能多显示一个字。
const ELLIPSIS: &str = "…";

/// 一个候选**实际要画的东西**。
///
/// 预先算好存起来，而不是绘制时现算：绘制每次重绘都走（按键、鼠标移动都会重绘），
/// 而且「排布时量的宽度」必须就是「画出来的东西的宽度」，否则命中区会和眼睛看到的
/// 对不上——那是输入法里最难查的一类 bug。
#[derive(Clone, Debug, PartialEq)]
struct DrawnItem {
    /// 主文字，含 `1.` 序号；超宽时已截断并加了省略号。
    text: String,
    /// 读音小字。截断过的项**不标读音**：那时字都显示不全，
    /// 再挤一小字读音只会更乱。
    hint: Option<&'static str>,
}

/// 单行里每一项的宽度预算。
///
/// 窗口最宽 `window_w`，这一行有 `n` 项，除了两侧留白和 `n-1` 道缝，
/// 剩下的平分。**不能只截断超长的那一项**：9 个候选里只有一个
/// 8 字词时，只截它仍然会把窗口撞出屏幕（其余八项也各占一截）。
fn item_budget(window_w: i32, n: usize) -> i32 {
    let n = n.max(1) as i32;
    ((window_w - 2 * pad() - (n - 1) * gap()) / n).max(crate::dpi::px(40))
}

/// 把一个候选装进 `cap` 宽度里，顺序是「先丢读音、再截字」。
///
/// `measure(text, small)` 由调用方提供：真机上是 GDI 量字，测试里是一把假尺子。
/// 这是故意抽成纯函数的：「截在哪一个字上」要能单测，而真机上截错一个字
/// 看不出来、也没人会为它报 bug。
fn fit_item(
    index: usize,
    word: &str,
    cap: i32,
    hint: Option<&'static str>,
    measure: &dyn Fn(&str, bool) -> i32,
) -> DrawnItem {
    let prefix = format!("{}.", index + 1);
    let body = crate::zhconv::apply(word);

    let total = |b: &str, h: Option<&str>| {
        let main = measure(&format!("{prefix}{b}"), false);
        main + match h {
            Some(h) => hint_gap() + measure(h, true),
            None => 0,
        }
    };

    // 1) 完整显示（带读音）能放下 —— 绝大多数候选走这一条。
    if total(&body, hint) <= cap {
        return DrawnItem {
            text: format!("{prefix}{body}"),
            hint,
        };
    }
    // 2) 放不下就先丢读音：它是注解，字才是内容。
    if total(&body, None) <= cap {
        return DrawnItem {
            text: format!("{prefix}{body}"),
            hint: None,
        };
    }
    // 3) 还放不下才截字。从少一个字开始试，直到加上省略号能放下。
    let chars: Vec<char> = body.chars().collect();
    let mut keep = chars.len().saturating_sub(1);
    loop {
        let cand: String = chars[..keep].iter().collect();
        let text = format!("{prefix}{cand}{ELLIPSIS}");
        if keep == 0 || measure(&text, false) <= cap {
            return DrawnItem { text, hint: None };
        }
        keep -= 1;
    }
}

/// 命中的**全表下标**（`(top + 行) * PAGE_SIZE + 列`）。
///
/// 多行之后命中判定是二维的：先按 y 定行、再按 x 定列。返回全表下标而不是
/// 「页内序号」，是因为多行下每一行属于不同的页——回调那边直接拿它去上屏，
/// 不必再问「用户点的是第几行」。
fn hit(st: &CandState, x: i32, y: i32) -> Option<usize> {
    if y < 0 {
        return None;
    }
    let row = y as usize / row_h().max(1) as usize;
    let spans = st.spans.get(row)?;
    let col = spans.iter().position(|(l, r)| x >= *l && x < *r)?;
    Some((st.top + row) * PAGE_SIZE + col)
}

/// 从鼠标消息的 `lParam` 取出客户区 x 坐标（低 16 位，带符号）。
///
/// **不要用 `GetCursorPos`**：那是「处理消息时」的鼠标位置，不是「消息发生那一刻」
/// 的位置。用户点得快一点、或者点击后顺手移开，就会算到相邻的候选上去。鼠标消息的
/// `lParam` 本身就是事件发生时的客户区坐标，Windows 已经替我们记好了。
fn client_x(l: LPARAM) -> i32 {
    (l.0 as u32 & 0xFFFF) as u16 as i16 as i32
}

/// 同 `client_x`，取 y（高 16 位）
fn client_y(l: LPARAM) -> i32 {
    ((l.0 as u32 >> 16) & 0xFFFF) as u16 as i16 as i32
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
        height_of(1),
        None,
        None,
        Some(HINSTANCE(hinst.0)),
        None,
    )
    .unwrap_or_default();
    WND.store(hwnd.0 as isize, Ordering::Release);
    hwnd
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {    match msg {
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        // 自绘背景，避免闪烁
        WM_ERASEBKGND => LRESULT(1),
        // 窗口被拖到缩放不同的屏幕上（少见，但 RDP / 切换缩放时会碰到）：
        // 按新 DPI 重排一遍。候选窗的宽高、行高、字号都跟 DPI 绑着，
        // 只重绘不重排的话字会溢出窗口。
        WM_DPICHANGED => {
            crate::dpi::update_for_window(hwnd);
            // 先把要重排的参数抄出来，**必须放开 STATE 的锁**再调 show——
            // show 自己也要锁它，重入会死锁。
            let args = STATE.lock().ok().and_then(|st| {
                if st.items.is_empty() {
                    return None;
                }
                Some((
                    st.items.clone(),
                    st.selected,
                    st.page,
                    st.rows,
                    st.x,
                    st.y,
                ))
            });
            if let Some((items, selected, page, rows, x, y)) = args {
                let refs: Vec<&str> = items.iter().map(|s| s.as_str()).collect();
                show(&refs, selected, page, rows, x, y);
            }
            LRESULT(0)
        }
        // 点候选窗不抢焦点，否则应用会失焦、组合态被断掉
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),

        WM_LBUTTONDOWN => {
            let idx = {
                let mut st = match STATE.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                let idx = hit(&st, client_x(l), client_y(l));
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
                let up = hit(&st, client_x(l), client_y(l));
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
                // `Ctrl`+左键 = 降权，普通左键 = 选词。
                //
                // 用消息自带的 `wParam` 键状态，而不是现在去问 `GetKeyState`：
                // 前者是**事件发生那一刻**的状态。用户点完立刻松开 Ctrl，问
                // `GetKeyState` 就会把这次点击当成普通左键——把词上了屏，
                // 而他按的是降权。
                //
                // `MK_CONTROL`（0x0008）在 windows-rs 0.62 里没导出（和 `SS_NOTIFY`
                // 一样），只能写字面值。
                if w.0 & MK_CONTROL != 0 {
                    invoke_demote(i);
                } else {
                    invoke_click(i);
                }
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
                hit(&st, client_x(l), client_y(l))
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
    for r in 0..st.rows {
        let items = page_slice(&st.items, st.top + r);
        if items.is_empty() {
            break;
        }
        // 活动行：数字键选的就是这一行。它是「最后一行」之外唯一的高亮线索，
        // 所以除了选中项那块底色，活动行整行的文字也用正常色，其余行压暗一档——
        // 多行摊开之后，用户得先知道「现在选的是哪一行」。
        let active = st.top + r == st.page;
        let row_top = crate::dpi::px(1) + r as i32 * row_h();
        let row_bottom = row_top + row_h();
        let ty = row_top + (row_h() - font_height(hdc)) / 2;

        // 画什么、占多宽，全部取自 layout 预先算好的结果（st.texts / st.hints）。
        // 这里**不能**重新 label(i, word) ——那样超宽截断就白做了，
        // 而且量出来的宽度会和 layout 里的命中区对不上。
        let mut x = pad();
        for i in 0..items.len() {
            let text = st
                .texts
                .get(r)
                .and_then(|row| row.get(i))
                .cloned()
                .unwrap_or_else(|| label(i, &items[i]));
            let hint = st.hints.get(r).and_then(|row| row.get(i)).copied().flatten();
            let wide: Vec<u16> = text.encode_utf16().collect();

            let mut size = SIZE::default();
            let _ = GetTextExtentPoint32W(hdc, &wide, &mut size);
            let (_, hw) = measure(hdc, &text, hint);
            let total = size.cx + if hint.is_some() { hint_gap() + hw } else { 0 };

            // st.selected 是**活动页内**的序号，不是全表序号
            let chosen = active && i == st.selected;
            if chosen {
                let hp = hit_pad();
                let hi = RECT {
                    left: x - hp,
                    top: row_top + crate::dpi::px(2),
                    right: x + total + hp,
                    bottom: row_bottom - crate::dpi::px(2),
                };
                crate::font::fill(hdc, &hi, p.accent);
                SetTextColor(hdc, p.accent_text);
            } else {
                SetTextColor(hdc, if active { p.text } else { p.dim });
            }

            let _ = TextOutW(hdc, x, ty, &wide);

            // 读音紧跟词后，小一号、灰一档：它是注解，不能和候选本身抢注意力
            if let Some(h) = hint {
                let hwide: Vec<u16> = h.encode_utf16().collect();
                let old = SelectObject(hdc, crate::font::small());
                SetTextColor(hdc, if chosen { p.accent_text } else { p.dim });
                // 比主文字低 2px，视觉上挂在词的下沿，不跟序号抢同一行
                let _ = TextOutW(hdc, x + size.cx + hint_gap(), ty + crate::dpi::px(2), &hwide);
                SelectObject(hdc, old);
                // 小字画完要把颜色恢复给下一个候选用（SetTextColor 不随字体还原）
                SetTextColor(hdc, if active { p.text } else { p.dim });
            }

            x += total + gap();
        }
    }
}

/// 主字号的高度。`GetTextExtentPoint32W` 量的是**串**，空串量不出东西，
/// 所以量一个固定的参考字形。
unsafe fn font_height(hdc: HDC) -> i32 {
    let probe: Vec<u16> = "国".encode_utf16().collect();
    let mut size = SIZE::default();
    let _ = GetTextExtentPoint32W(hdc, &probe, &mut size);
    size.cy.max(1)
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

/// 计算窗口宽度和每一行的横向命中区。
///
/// 绘制（`paint`）和命中判定（`hit`）都依赖这里的几何：paint 按同样的
/// `size.cx + gap()` 递增，高亮框左右各放宽 4px，所以命中区也用 ±4。
/// 多行时宽度取**最宽那一行**——每行各自按自己的内容排，行内不错位。
struct Layout {
    /// 窗口宽度（已夹到屏幕能装下的上限）
    w: i32,
    spans: Vec<Vec<(i32, i32)>>,
    /// 每项实际要画的字（超宽时是截断过的）
    texts: Vec<Vec<String>>,
    /// 每项要画的读音小字（超宽被砍掉时为 None）
    hints: Vec<Vec<Option<&'static str>>>,
}

unsafe fn layout(items: &[String], top: usize, rows: usize) -> Layout {
    let hdc = GetDC(None);
    if hdc.is_invalid() {
        return Layout {
            w: crate::dpi::px(320),
            spans: Vec::new(),
            texts: Vec::new(),
            hints: Vec::new(),
        };
    }
    SelectObject(hdc, crate::font::ui());

    let window_w = max_window_w();
    let mut width = 0;
    let mut spans = Vec::with_capacity(rows);
    let mut texts = Vec::with_capacity(rows);
    let mut hints = Vec::with_capacity(rows);
    for r in 0..rows {
        let list = page_slice(items, top + r);
        if list.is_empty() {
            break;
        }
        // 每一项的宽度预算按「这一行有几项」均分。见 item_budget 的注释：
        // 只截超长那一项是不够的。
        let cap = item_budget(window_w, list.len()).min(max_item_w());
        let mut x = pad();
        let mut row = Vec::with_capacity(list.len());
        let mut row_texts = Vec::with_capacity(list.len());
        let mut row_hints = Vec::with_capacity(list.len());
        for (i, word) in list.iter().enumerate() {
            let drawn = fit_item(i, word, cap, hint_of(word), &|t: &str, small: bool| {
                measure_plain(hdc, t, small)
            });
            let (mw, hw) = measure(hdc, &drawn.text, drawn.hint);
            let total = mw + if hw > 0 { hint_gap() + hw } else { 0 };
            row.push((x - hit_pad(), x + total + hit_pad()));
            row_texts.push(drawn.text);
            row_hints.push(drawn.hint);
            x += total + gap();
        }
        width = width.max(x - gap() + pad());
        spans.push(row);
        texts.push(row_texts);
        hints.push(row_hints);
    }
    let _ = ReleaseDC(None, hdc);
    Layout {
        w: width.max(crate::dpi::px(80)).min(window_w),
        spans,
        texts,
        hints,
    }
}

/// 只量一段文字，不考虑读音。给 [fit_item] 当尺子用。
unsafe fn measure_plain(hdc: HDC, text: &str, small: bool) -> i32 {
    let old = SelectObject(hdc, if small {
        crate::font::small()
    } else {
        crate::font::ui()
    });
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut size = SIZE::default();
    let _ = GetTextExtentPoint32W(hdc, &wide, &mut size);
    SelectObject(hdc, old);
    size.cx
}

/// 翻页时最多摊开几行、从哪一页开始。
///
/// `top = page - (rows - 1)`：活动页永远贴着**最后一行**，所以每按一次 `=` 整窗
/// 上滚一行（新的第 6 行从下面进来，第 1 行从上面出去），活动行始终停在底部——
/// 用户的眼睛不用在窗口里上下找「现在选的是哪一行」。
fn window_of(page: usize, rows: usize, pages: usize) -> (usize, usize) {
    let rows = rows.clamp(1, pages.max(1));
    (page.min(pages.saturating_sub(1)), page.saturating_sub(rows - 1))
}

/// 显示候选窗。`x` / `y` 是光标左下角的屏幕坐标。
///
/// `rows` = 这次最多摊开几行（1 = 老的单行横向布局）。实际行数会被「候选总页数」
/// 和「屏幕放不放得下」夹小。
pub fn show(items: &[&str], selected: usize, page: usize, rows: usize, x: i32, y: i32) {
    // 上次「窗口为什么没弹」的原因码，0 = 正常显示。只在这个码变化时写一行日志，
    // 按键路径上不会刷盘。见 `diag`。
    static REFUSE: AtomicU8 = AtomicU8::new(255);

    if items.is_empty() {
        // 空候选就收窗是正常路径（码还没拼完、打错了），不值得记
        REFUSE.store(0, Ordering::Relaxed);
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
        crate::dlog_change!(
            REFUSE,
            1,
            "candidate::show 被 no_popup 拦下（fullscreen={} fg={}）",
            crate::fullscreen::active(),
            crate::diag::fg_class()
        );
        hide();
        return;
    }
    // 全屏里**第一次**弹候选窗会让系统做一次画面重建：DWM 发现有置顶窗口盖在全屏
    // 程序上，就退出「全屏优化」，游戏得重建交换链——用户看到的是「游戏闪一下」。
    // 只发生一次，也躲不掉（任何置顶小窗都一样）。这里记一行，用户报「闪」时能
    // 直接确认是不是我们弹的窗造成的，而不用猜。
    {
        static FS_FIRST: AtomicBool = AtomicBool::new(false);
        let scene = crate::fullscreen::scene();
        if scene != crate::fullscreen::Scene::Normal && !FS_FIRST.swap(true, Ordering::Relaxed) {
            crate::diag::log(format_args!(
                "candidate::show 全屏下首次弹窗（scene={}），画面可能闪一次",
                scene.key()
            ));
        }
    }

    unsafe {
        if !ensure_class() {
            crate::dlog_change!(REFUSE, 2, "candidate::show：窗口类注册失败");
            return;
        }
    }

    let pages = items.len().div_ceil(PAGE_SIZE).max(1);

    // 按**光标所在那块屏**刷新 DPI：候选窗跟着光标走，主屏 150%、副屏 100%
    // 是常见配置。必须在 layout 之前刷——宽度和行高都要用新的比例算。
    crate::dpi::update_for_point(x, y);

    unsafe {
        let hwnd = ensure_window();
        if hwnd.is_invalid() {
            crate::dlog_change!(REFUSE, 3, "candidate::show：CreateWindowEx 失败");
            return;
        }

        // 屏幕放不下就砍行数：宁可少摊几行，也不能让窗口有一半在屏幕外
        // （在屏幕最下面打字是常态，候选只能往上长）。
        let sh = GetSystemMetrics(SM_CYSCREEN);
        let lift = crate::dpi::px(6);
        let fits = |n: usize| y + lift + height_of(n) <= sh || y - lift - height_of(n) >= 0;
        let mut rows = rows.min(pages);
        while rows > 1 && !fits(rows) {
            rows -= 1;
        }
        let (page, top) = window_of(page, rows.max(1), pages);

        let (w, h) = {
            let mut st = STATE.lock().unwrap();
            st.items.clear();
            st.items.extend(items.iter().map(|s| s.to_string()));
            st.selected = selected;
            st.page = page;
            st.top = top;
            st.rows = rows.max(1);
            st.x = x;
            st.y = y;
            st.pressed = None;
            // layout 只量字宽、不回调任何东西，可以持锁跑
            let lay = layout(&st.items, top, rows);
            // 候选不够一屏时行数会被截短，窗口高度要跟着缩，否则下半截是空白
            st.rows = lay.spans.len().max(1);
            st.spans = lay.spans;
            st.texts = lay.texts;
            st.hints = lay.hints;
            (lay.w, height_of(st.rows))
        };

        // 贴光标下方 6px，并夹在屏幕内
        //
        // w 已经在 layout 里被 max_window_w() 夹过一次，所以这里的
        // px + w > sw 一般只会在「光标靠右」时成立。但保留这个夹子
        // 仍然必要——屏幕可能在弹窗的瞬间被拔掉（RDP、投屏）。
        let sw = GetSystemMetrics(SM_CXSCREEN);
        let lift = crate::dpi::px(6);
        let mut px = x;
        let mut py = y + lift;
        if px + w > sw {
            px = (sw - w).max(0);
        } else if px < 0 {
            px = 0;
        }
        if py + h > sh {
            // 下方放不下就翻到光标上方
            py = (y - h - lift).max(0);
        }

        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            px,
            py,
            w,
            h,
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
    clear_demote_click_handler();
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
        show(&["甲", "乙", "丙"], 0, 0, 1, 200, 200);

        let hwnd = unsafe { FindWindowW(CLASS_NAME, w!("")) }.unwrap_or_default();
        assert!(!hwnd.is_invalid(), "候选窗没建出来");

        // 取第一项的中心：命中区的几何就是布局算出来那份
        let (x1, x2) = {
            let st = STATE.lock().unwrap();
            *st.spans
                .first()
                .and_then(|r| r.first())
                .expect("spans 为空")
        };
        let px = (x1 + x2) / 2;
        // 第一行正中（行高 28，取 y = 15）
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
        clear_demote_click_handler();
    }

    /// 命中区是左闭右开；边界落在右端点时属于下一项，这是「不留 1px 重叠」的约定。
    #[test]
    fn hit_uses_half_open_intervals() {
        let mut st = CandState {
            items: vec!["甲".into(), "乙".into()],
            selected: 0,
            page: 0,
            top: 0,
            rows: 1,
            x: 0,
            y: 0,
            spans: vec![vec![(6, 30), (30, 54)]],
            texts: Vec::new(),
            hints: Vec::new(),
            pressed: None,
        };
        assert_eq!(hit(&st, 5, 15), None, "左边留白不该命中");
        assert_eq!(hit(&st, 6, 15), Some(0));
        assert_eq!(hit(&st, 29, 15), Some(0));
        assert_eq!(hit(&st, 30, 15), Some(1));
        assert_eq!(hit(&st, 54, 15), None, "右端点属于不存在的下一项");
        assert_eq!(hit(&st, 10, -1), None, "客户区上方不该命中");
        assert_eq!(hit(&st, 10, row_h() + 2), None, "只有一行时第二行是空白");
        st.spans.clear();
        assert_eq!(hit(&st, 10, 15), None);
    }

    /// 多行时命中判定要跨行还原成**全表下标**：第 2 行的第 1 项是第 9 + 0 个候选。
    ///
    /// 这块以前只按 x 判，多行之后如果不加 y 维度，点第 2 行会当成点了第 1 行——
    /// 上屏的字和鼠标下的字不是一个，比「点了没反应」还糟。
    #[test]
    fn hit_maps_rows_to_global_index() {
        let mut st = CandState {
            items: (0..27).map(|i| format!("w{i}")).collect(),
            selected: 0,
            page: 2,
            top: 0,
            rows: 3,
            x: 0,
            y: 0,
            spans: vec![
                vec![(6, 30), (30, 54)],
                vec![(6, 30), (30, 54)],
                vec![(6, 30), (30, 54)],
            ],
            texts: Vec::new(),
            hints: Vec::new(),
            pressed: None,
        };
        assert_eq!(hit(&st, 10, 15), Some(0), "第 1 行第 1 项 = 全表 0");
        assert_eq!(hit(&st, 10, row_h() + 2), Some(9), "第 2 行第 1 项 = 全表 9");
        assert_eq!(hit(&st, 40, row_h() * 2 + 2), Some(19), "第 3 行第 2 项 = 全表 19");
        // 滚过一屏之后 top 不为 0，下标要跟着带偏移
        st.top = 2;
        assert_eq!(hit(&st, 10, 15), Some(18), "top=2 时第 1 行是全表第 18 个");
    }

    /// 翻页窗口：活动页贴着最后一行，翻过一屏就整窗上滚。
    #[test]
    fn window_scrolls_one_row_at_a_time() {
        // 一共 10 页（90 个候选），摊开 5 行
        assert_eq!(window_of(0, 5, 10), (0, 0), "第 1 页：从第 1 行起");
        assert_eq!(window_of(1, 5, 10), (1, 0), "按一次 = 之后活动页是第 2 行");
        assert_eq!(window_of(4, 5, 10), (4, 0), "第 5 页正好填满最后一行");
        assert_eq!(window_of(5, 5, 10), (5, 1), "第 6 页：第 1 行出去、第 6 行进来");
        assert_eq!(window_of(9, 5, 10), (9, 5));
        // 候选页数比行数少：行数要缩到页数
        assert_eq!(window_of(1, 5, 2), (1, 0));
        // 单行模式永远显示活动页自己
        assert_eq!(window_of(3, 1, 10), (3, 3));
    }

    /// 把几行候选画到内存位图上存 BMP，人眼看布局。
    ///
    /// **不进 CI**：它不断言任何像素，只是把「小字提示、高亮框、候选间距、
    /// 多行的行距和活动行压暗」画出来。这类毛病（挤在一起、被裁掉、灰得看不见）
    /// 没有一条能用断言表达。
    ///
    ///     cargo test --lib snapshot -- --ignored --nocapture
    ///
    /// 产出 `target/candidate_snapshot.bmp`。
    #[test]
    #[ignore = "人工看布局用，不做断言"]
    fn snapshot_hint_layout() {
        static LOCK: Mutex<()> = Mutex::new(());
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());

        const ROWS: usize = 3;
        let words = [
            "重庆", "弹窗", "你好", "银行", "水雷屯", "我们", "的", "地", "得",
            "重庆", "弹窗", "你好", "银行", "水雷屯", "我们", "的", "地", "得",
            "重庆", "弹窗", "你好", "银行", "水雷屯", "我们", "的", "地", "得",
        ];
        {
            let mut st = STATE.lock().unwrap();
            st.items = words.iter().map(|s| s.to_string()).collect();
            st.selected = 1;
            st.page = 1;
            st.top = 0;
            st.rows = ROWS;
        }
        let items: Vec<String> = words.iter().map(|s| s.to_string()).collect();
        let lay = unsafe { layout(&items, 0, ROWS) };
        let w = lay.w;
        assert!(w > 100 && lay.spans.len() == ROWS);

        let h = height_of(ROWS);
        unsafe {
            let screen = GetDC(None);
            let mem = CreateCompatibleDC(Some(screen));
            let bmp = CreateCompatibleBitmap(screen, w, h);
            let old = SelectObject(mem, bmp.into());
            let rc = RECT {
                left: 0,
                top: 0,
                right: w,
                bottom: h,
            };
            paint_content(mem, &rc);

            let stride = ((w * 3 + 3) / 4 * 4) as usize;
            let mut bi = BITMAPINFO::default();
            bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bi.bmiHeader.biWidth = w;
            bi.bmiHeader.biHeight = -h; // 负数 = 自上而下，省得再翻转
            bi.bmiHeader.biPlanes = 1;
            bi.bmiHeader.biBitCount = 24;
            bi.bmiHeader.biCompression = BI_RGB.0;
            let mut pix = vec![0u8; stride * h as usize];
            GetDIBits(
                mem,
                bmp,
                0,
                h as u32,
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
            out.extend_from_slice(&(-h).to_le_bytes());
            out.extend_from_slice(&1u16.to_le_bytes());
            out.extend_from_slice(&24u16.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&(pix.len() as u32).to_le_bytes());
            out.extend_from_slice(&[0u8; 16]);
            out.extend_from_slice(&pix);

            let path = concat!(env!("CARGO_MANIFEST_DIR"), "/target/candidate_snapshot.bmp");
            std::fs::write(path, &out).expect("写截图失败");
            println!("截图：{path}（{w}x{h}）");
        }
    }

    // ---- 超宽保护 （纯逻辑，不碰 GDI） ----

    /// 假尺子：中文字算 13px、ASCII 算 7px、小字号打六折。
    ///
    /// 真机上的实际宽度取决于字体、DPI、渲染引擎，断言里写死一个数
    /// 只会在换个屏幕后变成假失败。这里只关心——`fit_item` 在给定尺子下行为对不对。
    fn ruler(text: &str, small: bool) -> i32 {
        let base: i32 = text
            .chars()
            .map(|c| if (c as u32) < 128 { 7 } else { 13 })
            .sum();
        if small {
            base * 6 / 10
        } else {
            base
        }
    }

    /// 一行剩多少宽度给每一项：去两侧留白、去 n-1 道缝后平分。
    #[test]
    fn item_budget_splits_row_evenly() {
        // 窗口 900，9 项：两侧留白 + 8 道缝后平分
        let b = item_budget(900, 9);
        assert!(b > 0 && b < 200, "预算要是一个合理的小数，得到 {b}");
        // 项数少就宽松：同一个窗口下 3 项比 9 项宽
        assert!(item_budget(900, 3) > item_budget(900, 9));
        // 项数给 0 也不能除零
        assert!(item_budget(900, 0) >= 40);
        // 窗口极窄时不会算出负数预算
        assert_eq!(item_budget(10, 9), 40);
    }

    /// 放得下的候选一字不动（连读音一起保留）。
    #[test]
    fn fit_item_keeps_short_word_intact() {
        let d = fit_item(0, "重庆", 240, Some("chong'qing"), &ruler);
        assert_eq!(d.text, "1.重庆");
        assert_eq!(d.hint, Some("chong'qing"), "放得下就不该扛读音");
    }

    /// 只有读音挤不下时，先丢读音，不截字。
    #[test]
    fn fit_item_drops_hint_before_chars() {
        // "1.重庆" = 7+13+13 = 33；读音 "x"*20 = 140 * 0.6 = 84；加缝 5 ≈ 122。
        // 预算 60：字本身 33 放得下，带读音就不行。
        let d = fit_item(0, "重庆", 60, Some("xxxxxxxxxxxxxxxxxxxx"), &ruler);
        assert_eq!(d.text, "1.重庆", "字要保留");
        assert_eq!(d.hint, None, "读音是注解，先舍它");
    }

    /// 字本身就超宽时才截字，且尾巴是一个省略号。
    #[test]
    fn fit_item_truncates_long_word_with_ellipsis() {
        let word = "依法追究刑事责任"; // 8 字
        // 预算只够放 3 个字 + 序号 + 省略号：7 + 3*13 + 13 = 59
        let d = fit_item(0, word, 62, None, &ruler);
        assert!(d.text.ends_with('…'), "截断必须有省略号：{}", d.text);
        assert!(d.hint.is_none(), "截断了就别再挤读音");
        assert!(ruler(&d.text, false) <= 62, "截完必须真的放得下");
        // 截断后仍能看出是哪个词的开头
        assert!(d.text.starts_with("1.依法"));
    }

    /// 预算极窄时不能死循环、也不能返回空串。
    #[test]
    fn fit_item_survives_impossible_budget() {
        let d = fit_item(0, "重庆", 1, Some("chong'qing"), &ruler);
        assert!(!d.text.is_empty(), "再窄也得画点东西出来");
        assert_eq!(d.hint, None);
    }

    /// 序号要跟着全局位置走，不能每页都从 1 开始。
    #[test]
    fn fit_item_numbers_by_position() {
        let d = fit_item(8, "重庆", 240, None, &ruler);
        assert_eq!(d.text, "9.重庆");
    }
}
