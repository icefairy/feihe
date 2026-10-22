//! 配色：皮肤 → 调色板
//!
//! 三套内置配色（浅色 / 深色 / 护眼）。所有自绘窗口（候选窗、状态浮窗、软键盘）
//! 都从这里取色，保证换肤时整体一致。
//!
//! 注意 `COLORREF` 的字节序是 `0x00BBGGRR`（蓝绿红），不是 RGB。

use windows::Win32::Foundation::COLORREF;

use crate::config::Skin;

/// 从 `#RRGGBB` 习惯的写法构造 COLORREF
const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF((r as u32) | ((g as u32) << 8) | ((b as u32) << 16))
}

#[derive(Clone, Copy)]
pub struct Palette {
    /// 窗口底色
    pub bg: COLORREF,
    /// 边框
    pub border: COLORREF,
    /// 主文字
    pub text: COLORREF,
    /// 次要文字（未激活的状态）
    pub dim: COLORREF,
    /// 选中项底色
    pub accent: COLORREF,
    /// 选中项文字
    pub accent_text: COLORREF,
    /// 激活态强调色（中/全角 等亮起的状态）
    pub on: COLORREF,
    /// 按钮悬停底色
    pub hover: COLORREF,
}

pub fn of(skin: Skin) -> Palette {
    match skin {
        Skin::Light => Palette {
            bg: rgb(0xFF, 0xFF, 0xFF),
            border: rgb(0xC8, 0xC8, 0xC8),
            text: rgb(0x20, 0x20, 0x20),
            dim: rgb(0x78, 0x78, 0x78),
            accent: rgb(0xD9, 0xE8, 0xFB),
            accent_text: rgb(0x10, 0x30, 0x5A),
            on: rgb(0x0B, 0x63, 0xCE),
            hover: rgb(0xEE, 0xF3, 0xFA),
        },
        Skin::Dark => Palette {
            bg: rgb(0x1F, 0x1F, 0x1F),
            border: rgb(0x3C, 0x3C, 0x3C),
            text: rgb(0xE8, 0xE8, 0xE8),
            dim: rgb(0x96, 0x96, 0x96),
            accent: rgb(0x2D, 0x4A, 0x6B),
            accent_text: rgb(0xFF, 0xFF, 0xFF),
            on: rgb(0x6C, 0xB4, 0xFF),
            hover: rgb(0x2A, 0x2A, 0x2A),
        },
        Skin::Eye => Palette {
            bg: rgb(0xCC, 0xE8, 0xCF),
            border: rgb(0xA0, 0xC2, 0xA4),
            text: rgb(0x1E, 0x2E, 0x20),
            dim: rgb(0x4C, 0x60, 0x4E),
            accent: rgb(0xA9, 0xD2, 0xB0),
            accent_text: rgb(0x10, 0x24, 0x0F),
            on: rgb(0x14, 0x60, 0x1E),
            hover: rgb(0xC0, 0xDF, 0xC4),
        },
    }
}

/// 当前皮肤对应的调色板
pub fn current() -> Palette {
    of(crate::config::get().skin)
}
