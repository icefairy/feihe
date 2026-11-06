//! 符号 / 表情软键盘
//!
//! 9 列 × 5 行的自绘网格，点一下把字符插到当前文档光标处。底部一排页签在
//! 「符号」和「表情」之间切换，两页共用同一个网格尺寸，翻页不改窗口大小。
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
const ROWS: usize = 5;

/// 每页的格子数（一行 9 个、共 5 行）
const CELLS: usize = COLS * ROWS;

/// 底部页签的高度。只放两三个汉字，22px 够用，再高就喧宾夺主了。
const TAB_H: i32 = 22;

/// 一页内容。
///
/// 两页共用同一个 9×5 网格，只是数据源不同——这样翻页不需要重排窗口尺寸，
/// 鼠标位置对应的格子编号也不用换算。
struct Page {
    /// 页签名（底部页签上显示的字）
    tab: &'static str,
    /// 这一页是不是表情（决定用哪个字体画）
    emoji: bool,
    cells: [&'static str; CELLS],
}

/// 中文标点 + 数学/箭头符号。
const PAGE_SYMBOL: Page = Page {
    tab: "符号",
    emoji: false,
    cells: [
        "，", "。", "、", "；", "：", "？", "！", "…", "～",
        "“", "”", "‘", "’", "（", "）", "《", "》", "【",
        "＋", "－", "×", "÷", "＝", "≠", "≈", "≤", "≥",
        "％", "‰", "℃", "°", "±", "∞", "√", "∑", "·",
        "★", "☆", "○", "●", "△", "▲", "→", "←", "↓",
    ],
};

/// 常用 emoji，45 个一屏。
///
/// 不拿系统表情面板（`Win` + `.`）顶替：那是独立弹窗、会抢焦点，点完还得等它
/// 收回去，打字的节奏完全断掉。这里跟着输入法自己的面板走，点一下直接上屏，
/// 和敲一个字是同一个动作。
///
/// 只做一屏不做分类翻页：真正要找冷门表情的人会用系统面板，那里有搜索；
/// 输入法里放 45 个高频的就够，多出来的每一行都是维护成本。
///
/// 注意 GDI 画出来是**单色**轮廓（彩色 emoji 要 DirectWrite 的 COLR 支持）。
/// 这是有意的取舍：单色看得出是哪个表情，而引入 DirectWrite 渲染要几百行
/// 排版代码，为一个 45 格的面板不值。
const PAGE_EMOJI: Page = Page {
    tab: "表情",
    emoji: true,
    cells: [
        "😀", "😂", "😊", "😍", "😎", "🤔", "😭",
        "😡", "😴", "🙏", "👍", "👎", "👌", "💪",
        "👀", "💋", "🔥", "✨", "❤️", "💔", "💕",
        "🎉", "🎁", "🌹", "🌻", "☀️", "🌙", "🐶",
        "🐱", "🐰", "🐼", "🐟", "🌸", "🌳", "🍎",
        "🍕", "☕", "🍺", "🎂", "💡", "📱", "💻",
        "🎵", "🚀", "✅",
    ],
};

const PAGES: [&Page; 2] = [&PAGE_SYMBOL, &PAGE_EMOJI];

const WIDTH: i32 = PAD * 2 + CELL_W * COLS as i32;
/// 内容区高度（不含页签）
const BODY_H: i32 = PAD * 2 + CELL_H * ROWS as i32;
const HEIGHT: i32 = BODY_H + TAB_H;

static WND: AtomicIsize = AtomicIsize::new(0);
static CLASS_REGISTERED: AtomicBool = AtomicBool::new(false);
/// 悬停的格子编号（-1 = 没有）
static HOVER: AtomicI32 = AtomicI32::new(-1);
/// 当前页号
static PAGE: AtomicI32 = AtomicI32::new(0);

fn hwnd() -> HWND {
    let raw = WND.load(Ordering::Acquire);
    HWND(raw as *mut core::ffi::c_void)
}

/// 当前页
fn page() -> &'static Page {
    let i = PAGE.load(Ordering::Acquire).clamp(0, PAGES.len() as i32 - 1) as usize;
    PAGES[i]
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

/// 页签的矩形
fn tab_rect(i: usize) -> RECT {
    let w = (WIDTH - PAD * 2) / PAGES.len() as i32;
    let left = PAD + i as i32 * w;
    RECT {
        left,
        top: BODY_H,
        // 最后一页吃掉除不尽的余数，右边不留缝
        right: if i + 1 == PAGES.len() {
            WIDTH - PAD
        } else {
            left + w
        },
        bottom: HEIGHT,
    }
}

/// 命中的格子编号（不含页签区）
fn cell_at(x: i32, y: i32) -> Option<usize> {
    if y >= BODY_H || x < PAD || x >= WIDTH - PAD {
        return None;
    }
    (0..CELLS).find(|&i| {
        let r = cell_rect(i);
        x >= r.left && x < r.right && y >= r.top && y < r.bottom
    })
}

/// 命中的页签编号
fn tab_at(x: i32, y: i32) -> Option<usize> {
    if y < BODY_H {
        return None;
    }
    (0..PAGES.len()).find(|&i| {
        let r = tab_rect(i);
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
        return None;
    }
    let hinst = GetModuleHandleW(None).unwrap_or_default();
    let h = CreateWindowExW(
        WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
        CLASS_NAME,
        w!("符号与表情"),
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

    let pg = page();
    let hover = HOVER.load(Ordering::Acquire);

    // 表情用小一号的标记，让字形在 36px 的格子里留出呼吸空间；
    // 符号本身是单字，用正常字号才不显得空。
    let face = if pg.emoji { font::emoji() } else { font::ui() };
    SelectObject(hdc, face);
    SetBkMode(hdc, TRANSPARENT);

    // 内容区与页签之间画一条分隔线，让人看出下面是「切换」不是「第 6 行」
    let divider = RECT {
        left: PAD,
        top: BODY_H - 1,
        right: WIDTH - PAD,
        bottom: BODY_H,
    };
    font::fill(hdc, &divider, p.border);

    for (i, sym) in pg.cells.iter().enumerate() {
        let r = cell_rect(i);
        if hover == i as i32 {
            font::fill(hdc, &r, p.accent);
            font::text_center(hdc, &r, sym, p.accent_text);
        } else {
            font::text_center(hdc, &r, sym, p.text);
        }
    }

    // 页签：当前页用强调色，其余压暗
    SelectObject(hdc, font::small());
    for (i, pg) in PAGES.iter().enumerate() {
        let r = tab_rect(i);
        let color = if i as i32 == PAGE.load(Ordering::Acquire) {
            p.on
        } else {
            p.dim
        };
        font::text_center(hdc, &r, pg.tab, color);
    }

    let _ = EndPaint(h, &ps);
}

unsafe extern "system" fn wndproc(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let mx = || (l.0 & 0xFFFF) as i16 as i32;
    let my = || ((l.0 >> 16) & 0xFFFF) as i16 as i32;
    match msg {
        WM_PAINT => {
            paint(h);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_MOUSEMOVE => {
            let (x, y) = (mx(), my());
            // 悬停只标内容格；页签不跟随高亮，避免「一不小心就切了页」
            let idx = cell_at(x, y).map(|i| i as i32).unwrap_or(-1);
            if HOVER.swap(idx, Ordering::AcqRel) != idx {
                let _ = InvalidateRect(Some(h), None, true);
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            // 表情值当是「选中」而不是「输入」，一旦做成按下即上屏，右键预览就打不上补丁
            let (x, y) = (mx(), my());
            if let Some(i) = tab_at(x, y) {
                if PAGE.load(Ordering::Acquire) != i as i32 {
                    PAGE.store(i as i32, Ordering::Release);
                    HOVER.store(-1, Ordering::Release);
                    let _ = InvalidateRect(Some(h), None, true);
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = (mx(), my());
            if let Some(idx) = cell_at(x, y) {
                let sym = page().cells[idx];
                let _ = insert::insert_text(sym);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(h, msg, w, l),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 两页必须都是满格——少一个 `""` 就会画出空白的格子，
    /// 而那看着像「这个位置可以点但没反应」。
    #[test]
    fn every_page_is_full_and_has_no_blanks() {
        for pg in PAGES {
            for (i, c) in pg.cells.iter().enumerate() {
                assert!(!c.is_empty(), "{}第 {i} 格是空的", pg.tab);
            }
        }
    }

    /// 同一页内不能有重复字符：重复的格子点哪个都一样，纯属浪费位置。
    #[test]
    fn no_duplicate_cells_within_a_page() {
        for pg in PAGES {
            for i in 0..pg.cells.len() {
                for j in (i + 1)..pg.cells.len() {
                    assert_ne!(
                        pg.cells[i], pg.cells[j],
                        "{}页第 {i} 和第 {j} 格重复",
                        pg.tab
                    );
                }
            }
        }
    }

    /// 页签名不能重复，否则「点了哪个页签」没法用文字说明。
    #[test]
    fn tab_labels_are_distinct() {
        for i in 0..PAGES.len() {
            for j in (i + 1)..PAGES.len() {
                assert_ne!(PAGES[i].tab, PAGES[j].tab);
            }
        }
    }

    /// 格子不重叠、也不越界。重叠会造成「点一个格子却插了旁边的字符」。
    #[test]
    fn cells_tile_without_overlap() {
        for i in 0..CELLS {
            let a = cell_rect(i);
            assert!(a.right <= WIDTH - PAD && a.bottom <= BODY_H);
            for j in (i + 1)..CELLS {
                let b = cell_rect(j);
                let apart = a.right <= b.left
                    || b.right <= a.left
                    || a.bottom <= b.top
                    || b.bottom <= a.top;
                assert!(apart, "格 {i} 与格 {j} 重叠");
            }
        }
    }

    /// 页签铺满底部且首尾贴合，中间不留缝。
    #[test]
    fn tabs_cover_the_bottom_strip() {
        assert_eq!(tab_rect(0).left, PAD);
        assert_eq!(tab_rect(PAGES.len() - 1).right, WIDTH - PAD);
        for i in 1..PAGES.len() {
            assert_eq!(tab_rect(i).left, tab_rect(i - 1).right, "页签之间有缝");
        }
        for i in 0..PAGES.len() {
            assert_eq!(tab_rect(i).top, BODY_H);
            assert_eq!(tab_rect(i).bottom, HEIGHT);
        }
    }

    /// 页签区与内容区互不吃掉对方的点击。
    #[test]
    fn tabs_and_cells_do_not_steal_each_other() {
        // 内容区最下面一格的中间点：只能命中格子
        let last = cell_rect(CELLS - 1);
        let (x, y) = ((last.left + last.right) / 2, (last.top + last.bottom) / 2);
        assert_eq!(cell_at(x, y), Some(CELLS - 1));
        assert_eq!(tab_at(x, y), None);

        // 第一个页签的中间点：只能命中页签
        let t = tab_rect(0);
        let (x, y) = ((t.left + t.right) / 2, (t.top + t.bottom) / 2);
        assert_eq!(tab_at(x, y), Some(0));
        assert_eq!(cell_at(x, y), None);
    }

    /// 左右留白里点不出东西来（边框外沿不该算作格子）。
    #[test]
    fn padding_is_inert() {
        assert_eq!(cell_at(0, 20), None);
        assert_eq!(cell_at(WIDTH - 1, 20), None);
        assert_eq!(cell_at(20, 0), None);
    }

    /// 页号越界要夹回合法范围，不能让 `page()` 索引越界 panic。
    #[test]
    fn page_index_is_clamped() {
        // 直接验证 `clamp` 的边界意图（不碰全局原子，免得和别的测试抢状态）
        for raw in [-5i32, -1, 0, 1, 2, 99] {
            let i = raw.clamp(0, PAGES.len() as i32 - 1) as usize;
            assert!(i < PAGES.len());
        }
    }
}
