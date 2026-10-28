//! 读剪贴板里的纯文本。**只给快捷命令 `/fc` 用**（反查复制内容的音和形码）。
//!
//! ## 为什么自己开一次剪贴板
//!
//! 输入法进程一般都在「前台宿主进程」里，剪贴板随时可能被别的进程占着
//! （资源管理器复制文件、Office 复制大段内容时都会 `OpenClipboard` 一会儿才放）。
//! `OpenClipboard` 失败是常态而不是异常，所以这里短重试几次（3 × 5ms）再放弃，
//! 而不是一失败就报空——用户复制完立刻敲 `/fc` 正好撞上占用窗口的概率不低。
//!
//! ## 为什么有长度上限
//!
//! `CF_UNICODETEXT` 是一个以 NUL 结尾的 `wchar_t` 数组。理论上有 `GlobalSize`
//! 兜底，但坏掉的剪贴板内容（大小写错、别的程序塞了畸形数据）不该让输入法去
//! 扫描几十 MB 内存——反查最多也就处理一行字。所以先按 `GlobalSize` 截，再按
//! NUL 截，两道上限取小的那个。

use windows::Win32::Foundation::{HGLOBAL, HANDLE};
use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;

/// 剪贴板文本最多读这么多 `wchar`。反查是给人看的，一行字绰绰有余。
const MAX_CHARS: usize = 8192;

/// 取剪贴板里的纯文本；剪贴板没有文字（图片、文件）、或打不开时返回 `None`。
pub fn text() -> Option<String> {
    unsafe {
        // 被别的进程占着就短重试：复制完立刻按 `/fc` 是常见时序
        let mut opened = false;
        for _ in 0..3 {
            if OpenClipboard(None).is_ok() {
                opened = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if !opened {
            return None;
        }
        let out = read_unicode();
        let _ = CloseClipboard();
        out
    }
}

unsafe fn read_unicode() -> Option<String> {
    // 没有 CF_UNICODETEXT 说明剪贴板里不是文字（图片 / 文件列表），直接放弃
    let handle: HANDLE = GetClipboardData(CF_UNICODETEXT.0 as u32).ok()?;
    if handle.is_invalid() {
        return None;
    }
    let hglobal = HGLOBAL(handle.0);
    let p = GlobalLock(hglobal) as *const u16;
    if p.is_null() {
        return None;
    }
    // 两道上限：GlobalSize（字节 → wchar）+ MAX_CHARS
    let cap = (GlobalSize(hglobal) / 2).min(MAX_CHARS);
    let mut len = 0usize;
    while len < cap && *p.add(len) != 0 {
        len += 1;
    }
    let s = if len == 0 {
        String::new()
    } else {
        String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
    };
    let _ = GlobalUnlock(hglobal);
    Some(s)
}
