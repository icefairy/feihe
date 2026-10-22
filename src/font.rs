//! 自绘窗口共用的字体与画笔小工具
//!
//! 全系统默认 GUI 字体是宋体 9pt，在候选窗/状态浮窗里显得又小又毛。
//! 这里统一用「微软雅黑 UI」13px，找不到时 GDI 会自动回退到系统字体。

use std::sync::atomic::{AtomicIsize, Ordering};

use windows::core::*;
use windows::Win32::Foundation::{COLORREF, RECT, SIZE};
use windows::Win32::Graphics::Gdi::*;

static FONT: AtomicIsize = AtomicIsize::new(0);

/// 惰性创建并缓存 UI 字体。返回的句柄由本模块持有，调用方**不要** DeleteObject。
pub fn ui() -> HGDIOBJ {
    let raw = FONT.load(Ordering::Acquire);
    if raw != 0 {
        return HGDIOBJ(raw as *mut core::ffi::c_void);
    }
    let f = unsafe {
        CreateFontW(
            -13,
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
    FONT.store(f.0 as isize, Ordering::Release);
    f
}

/// 进程退出前释放字体句柄（DLL 卸载时调用，避免 GDI 句柄泄漏）
pub fn destroy() {
    let raw = FONT.swap(0, Ordering::AcqRel);
    if raw != 0 {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(raw as *mut core::ffi::c_void));
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
