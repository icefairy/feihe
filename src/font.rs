//! 自绘窗口共用的字体与画笔小工具
//!
//! 全系统默认 GUI 字体是宋体 9pt，在候选窗/状态浮窗里显得又小又毛。
//! 这里统一用「微软雅黑 UI」13px，找不到时 GDI 会自动回退到系统字体。

use std::sync::atomic::{AtomicU64, Ordering};

use windows::core::*;
use windows::Win32::Foundation::{COLORREF, RECT, SIZE};
use windows::Win32::Graphics::Gdi::*;

/// 缓存的结构：低 32 位是**字体句柄**，高 32 位是**建它时的 DPI**。
///
/// 只缓存一个句柄是不够的——DPI 变了（窗口拖到另一块屏、用户改了缩放）字号就得
/// 跟着变，但旧句柄又必须留着给「已经拿出去、还没 `SetTextColor` 完」的绘制。
/// 这里用「记录 (句柄, DPI)」的方式在 [`cached`] 里比对：不再匹配就换一个新的，
/// 旧的在 [`destroy`] 里统一释放。
///
/// 打包成一个 `AtomicU64` 而不是两个原子量：绘制路径每帧都要读，
/// 而它只在 DPI 变化时才写。
///
/// **必须是 `u64` 而不是 `isize`**：32 位构建里 `isize` 只有 32 位，
/// `<< 32` 会被 rustc 的 `arithmetic_overflow` 直接判为编译错误
/// （0.3.14 首次出 x86 包时就是这么挂的）。句柄本身在两份构建里都不超过
/// 32 位，塞进 u64 低半段没有信息损失。
static FONT: AtomicU64 = AtomicU64::new(0);
static SMALL: AtomicU64 = AtomicU64::new(0);
static EMOJI: AtomicU64 = AtomicU64::new(0);

/// 把「句柄 + 建它时的 DPI」压成一个整数。
fn packed(h: HGDIOBJ, dpi: u32) -> u64 {
    ((dpi as u64) << 32) | (h.0 as usize as u32 as u64)
}

fn unpack(raw: u64) -> (HGDIOBJ, u32) {
    let h = (raw as u32) as usize as *mut core::ffi::c_void;
    (HGDIOBJ(h), (raw >> 32) as u32)
}

/// 缓存命中：已经有句柄，且是给当前 DPI 建的。
fn cached(slot: &AtomicU64) -> Option<HGDIOBJ> {
    let raw = slot.load(Ordering::Acquire);
    if raw == 0 {
        return None;
    }
    let (h, d) = unpack(raw);
    (d == crate::dpi::dpi()).then_some(h)
}

/// 惰性创建并缓存 UI 字体。返回的句柄由本模块持有，调用方**不要** DeleteObject。
pub fn ui() -> HGDIOBJ {
    if let Some(h) = cached(&FONT) {
        return h;
    }
    let dpi = crate::dpi::dpi();
    let f = unsafe {
        CreateFontW(
            -crate::dpi::scale(13, dpi),
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            DEFAULT_PITCH.0 as u32,
            w!("Microsoft YaHei UI"),
        )
    };
    let f = if f.is_invalid() {
        unsafe { HGDIOBJ(GetStockObject(DEFAULT_GUI_FONT).0) }
    } else {
        HGDIOBJ(f.0)
    };
    FONT.store(packed(f, dpi), Ordering::Release);
    f
}

/// 小一号的字体：候选窗里的读音提示、状态浮窗的次要信息。
///
/// 比主字体矮 3px 而不是缩成 10px——雅黑在 10px 以下抗锯齿会糊成一团，
/// 拼音本来就是给人「顺手看一眼」的，糊了不如不显示。
pub fn small() -> HGDIOBJ {
    if let Some(h) = cached(&SMALL) {
        return h;
    }
    let dpi = crate::dpi::dpi();
    let f = unsafe {
        CreateFontW(
            -crate::dpi::scale(12, dpi),
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            DEFAULT_PITCH.0 as u32,
            w!("Microsoft YaHei UI"),
        )
    };
    let f = if f.is_invalid() {
        unsafe { HGDIOBJ(GetStockObject(DEFAULT_GUI_FONT).0) }
    } else {
        HGDIOBJ(f.0)
    };
    SMALL.store(packed(f, dpi), Ordering::Release);
    f
}

/// emoji 字体。
///
/// 颜色 emoji 需要 DirectWrite 的 COLR 表支持，GDI 只能画出单色轮廓。
/// 单色也能认出是哪个表情，而为一个 45 格的面板引入
/// 几百行 DirectWrite 排版代码不划算——这是有意的取舍。
///
/// 16px：比主字号大一档，让字形在 36px 的格子里留出呼吸空间；
/// 再大就会溢到格子边。找不到「Segoe UI Emoji」时 GDI 自动回退到系统字体。
pub fn emoji() -> HGDIOBJ {
    if let Some(h) = cached(&EMOJI) {
        return h;
    }
    let dpi = crate::dpi::dpi();
    let f = unsafe {
        CreateFontW(
            -crate::dpi::scale(16, dpi),
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            DEFAULT_PITCH.0 as u32,
            w!("Segoe UI Emoji"),
        )
    };
    let f = if f.is_invalid() {
        unsafe { HGDIOBJ(GetStockObject(DEFAULT_GUI_FONT).0) }
    } else {
        HGDIOBJ(f.0)
    };
    EMOJI.store(packed(f, dpi), Ordering::Release);
    f
}

/// 进程退出前释放字体句柄（DLL 卸载时调用，避免 GDI 句柄泄漏）
pub fn destroy() {
    for slot in [&FONT, &SMALL, &EMOJI] {
        let raw = slot.swap(0, Ordering::AcqRel);
        if raw != 0 {
            let (h, _) = unpack(raw);
            unsafe {
                let _ = DeleteObject(h);
            }
        }
    }
}

/// 用纯色填充矩形
pub unsafe fn fill(hdc: HDC, rc: &RECT, color: COLORREF) {
    let brush = CreateSolidBrush(color);
    FillRect(hdc, rc, brush);
    let _ = DeleteObject(HGDIOBJ(brush.0));
}

/// 单像素描边
pub unsafe fn frame(hdc: HDC, rc: &RECT, color: COLORREF) {
    let brush = CreateSolidBrush(color);
    FrameRect(hdc, rc, brush);
    let _ = DeleteObject(HGDIOBJ(brush.0));
}

/// 在矩形内居中画一行文字
pub unsafe fn text_center(hdc: HDC, rc: &RECT, s: &str, color: COLORREF) {
    let wide: Vec<u16> = s.encode_utf16().collect();
    let mut size = SIZE::default();
    let _ = GetTextExtentPoint32W(hdc, &wide, &mut size);
    SetTextColor(hdc, color);
    let x = rc.left + (rc.right - rc.left - size.cx) / 2;
    let y = rc.top + (rc.bottom - rc.top - size.cy) / 2;
    let _ = TextOutW(hdc, x, y, &wide);
}
