//! 设置窗：双拼方案 / 辅助码方案 / 皮肤 / 全角 / 浮窗跟随 / 词库管理
//!
//! 用原生 BUTTON / STATIC 控件，不做自绘——设置窗是低频交互，系统控件自带键盘
//! 导航和辅助功能支持，比自绘划算。代价是不跟随皮肤配色（只影响这一个窗口）。
//!
//! 单选按钮靠 Win32 的 `WS_GROUP` 自动分组互斥：每组第一个按钮带 `WS_GROUP`，
//! 组在下一个带 `WS_GROUP` 的控件处结束。所以创建顺序必须和视觉顺序一致。

use std::sync::atomic::{AtomicIsize, Ordering};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{COLOR_WINDOW, HBRUSH};
use windows::Win32::System::LibraryLoader::{
    GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};
use windows::Win32::UI::Controls::Dialogs::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::assist::AssistMode;
use crate::config::{self, Skin};
use crate::dict;
use crate::font;
use crate::shuangpin::Scheme;
use crate::statusbar;

const CLASS_NAME: PCWSTR = w!("FeiheSettings");

const W: i32 = 460;
const H: i32 = 424;

// 控件 ID
const ID_SCHEME_XH: i32 = 101;
const ID_SCHEME_MS: i32 = 102;
const ID_ASSIST_OFF: i32 = 111;
const ID_ASSIST_ZRM: i32 = 112;
const ID_ASSIST_XH: i32 = 113;
const ID_SKIN_LIGHT: i32 = 121;
const ID_SKIN_DARK: i32 = 122;
const ID_SKIN_EYE: i32 = 123;
const ID_FULLWIDTH: i32 = 131;
const ID_FOLLOW: i32 = 132;
const ID_STATS: i32 = 141;
const ID_NOTE: i32 = 142;
const ID_IMPORT: i32 = 151;
const ID_EXPORT: i32 = 152;
const ID_CLOSE: i32 = 153;

static WND: AtomicIsize = AtomicIsize::new(0);

fn hwnd() -> HWND {
    let raw = WND.load(Ordering::Acquire);
    HWND(raw as *mut core::ffi::c_void)
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn mk(
    parent: HWND,
    class: PCWSTR,
    text: &str,
    style: WINDOW_STYLE,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: i32,
) -> HWND {
    let wide = to_wide(text);
    let child = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        class,
        PCWSTR(wide.as_ptr()),
        WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0) | style,
        x,
        y,
        w,
        h,
        Some(parent),
        Some(HMENU(id as isize as *mut core::ffi::c_void)),
        None,
        None,
    )
    .unwrap_or_default();
    if !child.is_invalid() {
        SendMessageW(
            child,
            WM_SETFONT,
            Some(WPARAM(font::ui().0 as usize)),
            Some(LPARAM(1)),
        );
    }
    child
}

fn label(parent: HWND, text: &str, x: i32, y: i32, w: i32, id: i32) -> HWND {
    unsafe { mk(parent, w!("STATIC"), text, WINDOW_STYLE(0), x, y, w, 20, id) }
}

fn radio(parent: HWND, text: &str, x: i32, y: i32, w: i32, id: i32, group_start: bool) -> HWND {
    let mut style = WINDOW_STYLE(BS_AUTORADIOBUTTON as u32);
    if group_start {
        style |= WS_GROUP;
    }
    unsafe { mk(parent, w!("BUTTON"), text, style, x, y, w, 22, id) }
}

fn check(parent: HWND, text: &str, x: i32, y: i32, w: i32, id: i32) -> HWND {
    unsafe {
        mk(
            parent,
            w!("BUTTON"),
            text,
            WINDOW_STYLE(BS_AUTOCHECKBOX as u32) | WS_GROUP,
            x,
            y,
            w,
            22,
            id,
        )
    }
}

fn push(parent: HWND, text: &str, x: i32, y: i32, w: i32, h: i32, id: i32) -> HWND {
    unsafe {
        mk(
            parent,
            w!("BUTTON"),
            text,
            WINDOW_STYLE(BS_PUSHBUTTON as u32) | WS_GROUP | WS_TABSTOP,
            x,
            y,
            w,
            h,
            id,
        )
    }
}

unsafe fn checked(h: HWND) -> bool {
    SendMessageW(h, BM_GETCHECK, None, None).0 != 0
}

unsafe fn set_checked(parent: HWND, id: i32, on: bool) {
    let h = GetDlgItem(Some(parent), id).unwrap_or_default();
    if !h.is_invalid() {
        SendMessageW(h, BM_SETCHECK, Some(WPARAM(if on { 1 } else { 0 })), None);
    }
}

unsafe fn set_text(parent: HWND, id: i32, text: &str) {
    let h = GetDlgItem(Some(parent), id).unwrap_or_default();
    if !h.is_invalid() {
        let _ = SetWindowTextW(h, PCWSTR(to_wide(text).as_ptr()));
    }
}

/// 按当前配置刷新所有控件的勾选状态与词库统计
unsafe fn sync(h: HWND) {
    let cfg = config::get();
    set_checked(h, ID_SCHEME_XH, cfg.scheme == Scheme::Xiaohe);
    set_checked(h, ID_SCHEME_MS, cfg.scheme == Scheme::Ms);
    set_checked(h, ID_ASSIST_OFF, cfg.assist_mode == AssistMode::Off);
    set_checked(h, ID_ASSIST_ZRM, cfg.assist_mode == AssistMode::Ziranma);
    set_checked(h, ID_ASSIST_XH, cfg.assist_mode == AssistMode::Xiaohe);
    set_checked(h, ID_SKIN_LIGHT, cfg.skin == Skin::Light);
    set_checked(h, ID_SKIN_DARK, cfg.skin == Skin::Dark);
    set_checked(h, ID_SKIN_EYE, cfg.skin == Skin::Eye);
    set_checked(h, ID_FULLWIDTH, cfg.fullwidth);
    set_checked(h, ID_FOLLOW, cfg.bar_follow);

    let (total, user, scheme) = dict::stats();
    set_text(
        h,
        ID_STATS,
        &format!(
            "词库：共 {total} 条，其中用户词 {user} 条 · 当前编码方案：{}",
            scheme.label()
        ),
    );
}

/// 切换双拼方案后重建词库索引（约 30~80ms，可以接受）
fn apply_scheme(scheme: Scheme) {
    config::update(|c| c.scheme = scheme);
    dict::set_scheme(scheme);
    crate::candidate::hide();
}

fn pick_file(h: HWND, save: bool) -> Option<std::path::PathBuf> {
    let mut buf = [0u16; 1024];
    let filter: Vec<u16> = "词库文件 (*.txt)\0*.txt\0所有文件 (*.*)\0*.*\0\0"
        .encode_utf16()
        .collect();
    let title = if save { "导出词库" } else { "导入词库" };
    let title_w = to_wide(title);
    let defext = to_wide("txt");

    let mut ofn = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: h,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(buf.as_mut_ptr()),
        nMaxFile: buf.len() as u32,
        lpstrTitle: PCWSTR(title_w.as_ptr()),
        lpstrDefExt: PCWSTR(defext.as_ptr()),
        Flags: if save {
            OFN_EXPLORER | OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR
        } else {
            OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR
        },
        ..Default::default()
    };
    let ok = unsafe {
        if save {
            GetSaveFileNameW(&mut ofn).as_bool()
        } else {
            GetOpenFileNameW(&mut ofn).as_bool()
        }
    };
    if !ok {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(std::path::PathBuf::from(String::from_utf16_lossy(&buf[..len])))
}

unsafe fn on_command(h: HWND, id: i32) {
    match id {
        ID_SCHEME_XH => apply_scheme(Scheme::Xiaohe),
        ID_SCHEME_MS => apply_scheme(Scheme::Ms),
        ID_ASSIST_OFF => config::update(|c| c.assist_mode = AssistMode::Off),
        ID_ASSIST_ZRM => config::update(|c| c.assist_mode = AssistMode::Ziranma),
        ID_ASSIST_XH => config::update(|c| c.assist_mode = AssistMode::Xiaohe),
        ID_SKIN_LIGHT => config::update(|c| c.skin = Skin::Light),
        ID_SKIN_DARK => config::update(|c| c.skin = Skin::Dark),
        ID_SKIN_EYE => config::update(|c| c.skin = Skin::Eye),
        ID_FULLWIDTH => {
            let on = checked(GetDlgItem(Some(h), ID_FULLWIDTH).unwrap_or_default());
            config::update(|c| c.fullwidth = on);
        }
        ID_FOLLOW => {
            let on = checked(GetDlgItem(Some(h), ID_FOLLOW).unwrap_or_default());
            config::update(|c| {
                c.bar_follow = on;
                // 关掉跟随时，用当前位置作为固定位置，避免浮窗跳回屏幕角落
                if !on {
                    let bar = crate::statusbar::current_pos().unwrap_or((0, 0));
                    if bar != (0, 0) {
                        c.bar_x = bar.0;
                        c.bar_y = bar.1;
                    }
                }
            });
        }
        ID_IMPORT => {
            if let Some(path) = pick_file(h, false) {
                match dict::import_from(&path) {
                    Ok(n) => set_text(h, ID_NOTE, &format!("已导入 {n} 条新词。")),
                    Err(e) => set_text(h, ID_NOTE, &format!("导入失败：{e}")),
                }
            }
        }
        ID_EXPORT => {
            if let Some(path) = pick_file(h, true) {
                match dict::export_to(&path) {
                    Ok(n) => set_text(h, ID_NOTE, &format!("已导出 {n} 条词到 {path:?}")),
                    Err(e) => set_text(h, ID_NOTE, &format!("导出失败：{e}")),
                }
            }
        }
        ID_CLOSE => {
            let _ = ShowWindow(h, SW_HIDE);
            return;
        }
        _ => return,
    }

    // 皮肤 / 跟随 变更要立刻反映到浮窗和候选窗
    statusbar::refresh();
    crate::candidate::refresh_theme();
    sync(h);
}

unsafe fn build(h: HWND) {
    let mut y = 16;
    label(h, "双拼方案", 16, y, 90, 0);
    radio(h, "小鹤双拼", 110, y - 2, 100, ID_SCHEME_XH, true);
    radio(h, "微软双拼", 220, y - 2, 100, ID_SCHEME_MS, false);

    y += 40;
    label(h, "辅助码", 16, y, 90, 0);
    radio(h, "关闭", 110, y - 2, 70, ID_ASSIST_OFF, true);
    radio(h, "自然码辅助码", 190, y - 2, 120, ID_ASSIST_ZRM, false);
    radio(h, "小鹤音形", 320, y - 2, 100, ID_ASSIST_XH, false);

    y += 40;
    label(h, "皮肤", 16, y, 90, 0);
    radio(h, "浅色", 110, y - 2, 70, ID_SKIN_LIGHT, true);
    radio(h, "深色", 190, y - 2, 70, ID_SKIN_DARK, false);
    radio(h, "护眼", 270, y - 2, 70, ID_SKIN_EYE, false);

    y += 40;
    check(h, "全角标点 / 全角字符", 16, y - 2, 220, ID_FULLWIDTH);

    y += 30;
    check(h, "状态浮窗跟随光标（默认固定在屏幕右下角，可拖动）", 16, y - 2, 420, ID_FOLLOW);

    y += 40;
    label(h, "", 16, y, 428, ID_STATS);

    y += 34;
    push(h, "导入词库…", 16, y, 110, 28, ID_IMPORT);
    push(h, "导出词库…", 136, y, 110, 28, ID_EXPORT);

    y += 40;
    label(h, "导入格式：每行「词<TAB>全拼」，例如：你好<TAB>ni hao", 16, y, 428, ID_NOTE);

    // 快捷输入说明。码表从 quick 模块取，避免文档和实现对不上。
    let quick = crate::quick::commands()
        .iter()
        .map(|(k, name)| format!("\\{k} {name}"))
        .collect::<Vec<_>>()
        .join(" · ");
    label(h, &format!("快捷输入：{quick}"), 16, 306, 428, 0);

    push(h, "关闭", W - 106, 348, 90, 30, ID_CLOSE);
}

unsafe extern "system" fn wndproc(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            build(h);
            sync(h);
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = (w.0 & 0xFFFF) as i32;
            let code = ((w.0 >> 16) & 0xFFFF) as u32;
            // 只处理点击；忽略控件自身的通知噪音
            if code == BN_CLICKED || code == 0 {
                on_command(h, id);
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = ShowWindow(h, SW_HIDE);
            LRESULT(0)
        }
        WM_DESTROY => {
            WND.store(0, Ordering::Release);
            LRESULT(0)
        }
        _ => DefWindowProcW(h, msg, w, l),
    }
}

/// 取**本 DLL** 的模块句柄。
///
/// 不能用 `GetModuleHandleW(None)`——那个返回的是宿主进程的 EXE，图标资源在
/// feihe.dll 里，用 EXE 的句柄去 LoadIconW 只会拿到空（或者更糟：拿到宿主的同名资源）。
/// 这里按「函数地址反查所属模块」拿，和 DLL 被加载到哪里无关。
fn dll_module() -> Option<HMODULE> {
    unsafe {
        let mut m = HMODULE::default();
        let addr = dll_module as *const core::ffi::c_void;
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(addr as *const u16),
            &mut m,
        )
        .ok()?;
        Some(m)
    }
}

const IDI_FEIHE: usize = 101;

/// 从本 DLL 的资源里取图标。
///
/// `installer/feihe.rc` 里 `IDI_FEIHE ICON "assets/feihe.ico"` 编成了 RT_GROUP_ICON 101，
/// 所以要按序号取（`MAKEINTRESOURCE`）。取不到就返回 None，窗口退回系统默认图标，
/// 不能因为图标问题让设置窗打不开。
fn load_icon() -> Option<HICON> {
    unsafe {
        let hinst = dll_module()?;
        LoadIconW(Some(HINSTANCE(hinst.0)), PCWSTR(IDI_FEIHE as *const u16)).ok()
    }
}

unsafe fn create() -> Option<HWND> {
    let hinst = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default();
    let icon = load_icon();
    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wndproc),
        hInstance: HINSTANCE(hinst.0),
        hIcon: icon.unwrap_or_default(),
        lpszClassName: CLASS_NAME,
        hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as isize as *mut core::ffi::c_void),
        ..Default::default()
    };
    RegisterClassW(&wc);

    let h = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        CLASS_NAME,
        w!("飞鹤输入法 · 设置"),
        WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        W,
        H,
        None,
        None,
        Some(HINSTANCE(hinst.0)),
        None,
    )
    .unwrap_or_default();
    if h.is_invalid() {
        crate::trace!("settings: CreateWindowEx failed");
        return None;
    }

    // 类图标只管任务栏和 Alt+Tab；标题栏左上角那个小图标要单独 WM_SETICON 才换得掉
    if let Some(icon) = icon {
        SendMessageW(
            h,
            WM_SETICON,
            Some(WPARAM(ICON_SMALL as usize)),
            Some(LPARAM(icon.0 as isize)),
        );
        SendMessageW(
            h,
            WM_SETICON,
            Some(WPARAM(ICON_BIG as usize)),
            Some(LPARAM(icon.0 as isize)),
        );
    }
    Some(h)
}

/// 打开设置窗（已存在则激活）
pub fn open() {
    let existing = hwnd();
    if !existing.is_invalid() {
        unsafe {
            let _ = ShowWindow(existing, SW_SHOW);
            let _ = SetForegroundWindow(existing);
            sync(existing);
        }
        return;
    }
    unsafe {
        let Some(h) = create() else { return };
        WND.store(h.0 as isize, Ordering::Release);
        let _ = ShowWindow(h, SW_SHOW);
        let _ = SetForegroundWindow(h);
        sync(h);
    }
}

/// 别处改了配置（比如状态栏换肤）时同步控件
pub fn notify_config_changed() {
    let h = hwnd();
    if h.is_invalid() {
        return;
    }
    unsafe { sync(h) };
}

pub fn destroy() {
    let raw = WND.swap(0, Ordering::AcqRel);
    if raw == 0 {
        return;
    }
    unsafe {
        let _ = DestroyWindow(HWND(raw as *mut core::ffi::c_void));
    }
}
