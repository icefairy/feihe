//! 设置窗：输入模式 / 双拼方案 / 辅助码 / 皮肤 / 字形 / 全角 / 浮窗跟随 / 词库管理
//!
//! 用原生 BUTTON / STATIC 控件，不做自绘——设置窗是低频交互，系统控件自带键盘
//! 导航和辅助功能支持，比自绘划算。代价是不跟随皮肤配色（只影响这一个窗口）。
//!
//! 单选按钮靠 Win32 的 `WS_GROUP` 自动分组互斥：每组第一个按钮带 `WS_GROUP`，
//! 组在下一个带 `WS_GROUP` 的控件处结束。所以创建顺序必须和视觉顺序一致。
//!
//! 「皮肤」「全拼/双拼」原来在状态浮窗上，后来挪到这里：它们改一次就长期不变，
//! 占浮窗宽度不值得。浮窗只留随时要按的开关。

use std::sync::atomic::{AtomicIsize, Ordering};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{COLOR_WINDOW, HBRUSH};
use windows::Win32::System::LibraryLoader::{
    GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};
use windows::Win32::UI::Controls::Dialogs::*;
// EnableWindow 在 windows-rs 里归到了 KeyboardAndMouse 模块（跟着 user32 的分组走）
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::assist::AssistMode;
use crate::config::{self, PopupMode, Skin};
use crate::dict;
use crate::font;
use crate::shuangpin::Scheme;
use crate::statusbar;

const CLASS_NAME: PCWSTR = w!("FeiheSettings");

/// **客户区**尺寸。窗口尺寸由 `window_size()` 反算——标题栏高度随 DPI 和系统主题变，
/// 写死窗口高度会在高 DPI / 大字号下把最下面那排按钮截掉。
const W: i32 = 460;
const CLIENT_H: i32 = 810;

// 控件 ID
const ID_MODE_DS: i32 = 161;
const ID_MODE_FULL: i32 = 162;
const ID_DEF_CN: i32 = 163;
const ID_DEF_EN: i32 = 164;
const ID_SCHEME_XH: i32 = 101;
const ID_SCHEME_MS: i32 = 102;
const ID_ASSIST_OFF: i32 = 111;
const ID_ASSIST_ZRM: i32 = 112;
const ID_ASSIST_XH: i32 = 113;
const ID_ASSIST_DIRECT: i32 = 114;
const ID_SKIN_LIGHT: i32 = 121;
const ID_SKIN_DARK: i32 = 122;
const ID_SKIN_EYE: i32 = 123;
const ID_HANZI_SIMP: i32 = 171;
const ID_HANZI_TRAD: i32 = 172;
const ID_FULLWIDTH: i32 = 131;
const ID_FOLLOW: i32 = 132;
const ID_LEARN: i32 = 133;
const ID_LEARN_RESET: i32 = 134;
const ID_LEARN_HINT: i32 = 135;
const ID_POPUP_AUTO: i32 = 136;
const ID_POPUP_ALWAYS: i32 = 137;
const ID_POPUP_NEVER: i32 = 138;
const ID_READING: i32 = 139;
const ID_ROWS: i32 = 140;
const ID_STATS: i32 = 141;
const ID_NOTE: i32 = 142;
const ID_PIN_HINT: i32 = 143;
const ID_PIN_RESET: i32 = 144;
const ID_DATA_DIR: i32 = 145;
const ID_IMPORT: i32 = 151;
const ID_EXPORT: i32 = 152;
const ID_CLOSE: i32 = 153;
const ID_COMPILE: i32 = 154;

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

/// 下拉列表（行数这种「一串小整数」用它最省地方，不用排一长排单选按钮）
fn combo(parent: HWND, x: i32, y: i32, w: i32, id: i32) -> HWND {
    unsafe {
        // 四个参数的 h 对下拉列表是「展开后列表的高度」，不是控件高度
        mk(
            parent,
            w!("COMBOBOX"),
            "",
            WINDOW_STYLE(CBS_DROPDOWNLIST as u32) | WS_VSCROLL | WS_TABSTOP,
            x,
            y,
            w,
            200,
            id,
        )
    }
}

unsafe fn combo_add(h: HWND, text: &str) {
    let wide = to_wide(text);
    SendMessageW(
        h,
        CB_ADDSTRING,
        None,
        Some(LPARAM(wide.as_ptr() as isize)),
    );
}

/// 当前选中项；没选中（CB_ERR）返回 None
unsafe fn combo_pick(h: HWND) -> Option<usize> {
    let r = SendMessageW(h, CB_GETCURSEL, None, None).0;
    (r >= 0).then_some(r as usize)
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
    set_checked(h, ID_MODE_DS, !cfg.pinyin_mode);
    set_checked(h, ID_MODE_FULL, cfg.pinyin_mode);
    // 启动默认模式是设置项，和当前会话的中英状态无关（Shift 切换不落盘）
    set_checked(h, ID_DEF_CN, cfg.chinese);
    set_checked(h, ID_DEF_EN, !cfg.chinese);
    set_checked(h, ID_SCHEME_XH, cfg.scheme == Scheme::Xiaohe);
    set_checked(h, ID_SCHEME_MS, cfg.scheme == Scheme::Ms);
    set_checked(h, ID_ASSIST_OFF, cfg.assist_mode == AssistMode::Off);
    set_checked(h, ID_ASSIST_ZRM, cfg.assist_mode == AssistMode::Ziranma);
    set_checked(h, ID_ASSIST_XH, cfg.assist_mode == AssistMode::Xiaohe);
    set_checked(h, ID_SKIN_LIGHT, cfg.skin == Skin::Light);
    set_checked(h, ID_SKIN_DARK, cfg.skin == Skin::Dark);
    set_checked(h, ID_SKIN_EYE, cfg.skin == Skin::Eye);
    set_checked(h, ID_HANZI_SIMP, !cfg.traditional);
    set_checked(h, ID_HANZI_TRAD, cfg.traditional);
    set_checked(h, ID_FULLWIDTH, cfg.fullwidth);
    set_checked(h, ID_FOLLOW, cfg.bar_follow);
    set_checked(h, ID_POPUP_AUTO, cfg.popup == PopupMode::Auto);
    set_checked(h, ID_POPUP_ALWAYS, cfg.popup == PopupMode::Always);
    set_checked(h, ID_POPUP_NEVER, cfg.popup == PopupMode::Never);
    set_checked(h, ID_READING, cfg.reading);
    set_checked(h, ID_ASSIST_DIRECT, cfg.assist_direct);
    set_checked(h, ID_LEARN, cfg.learn);
    if let Ok(cb) = GetDlgItem(Some(h), ID_ROWS) {
        let idx = config::clamp_rows(cfg.candidate_rows) - config::ROWS_MIN;
        SendMessageW(cb, CB_SETCURSEL, Some(WPARAM(idx)), None);
    }
    let learned = crate::freq::len();
    let phrases = crate::phrase::len();
    set_text(
        h,
        ID_LEARN_HINT,
        &format!("已记 {learned} 个词频 · {phrases} 个自造词"),
    );
    // 数据目录写出来，用户才知道该备份哪个文件夹。装到只读目录时会回落
    // `%APPDATA%\Feihe`，这里显示的永远是**真实在用**的那个路径。
    set_text(
        h,
        ID_DATA_DIR,
        &format!(
            "数据目录：{}（压缩这个文件夹即备份）",
            config::dir().display()
        ),
    );
    // 条目少的时候直接把「码→词」列出来，用户不用去翻文件
    let pinned = crate::pin::list();
    let pin_text = match pinned.len() {
        0 => "置顶：无 · Ctrl+数字 置顶当前候选".to_string(),
        n if n <= 2 => format!(
            "置顶 {} · Ctrl+数字 切换",
            pinned
                .iter()
                .map(|(c, w)| format!("{c}→{w}"))
                .collect::<Vec<_>>()
                .join(" · ")
        ),
        n => format!("置顶 {n} 个编码 · Ctrl+数字 切换"),
    };
    set_text(h, ID_PIN_HINT, &pin_text);

    // 全拼模式下「双拼方案」没有意义，灰掉而不是隐藏——隐藏会让窗口内容跳动
    for id in [ID_SCHEME_XH, ID_SCHEME_MS] {
        if let Ok(ctl) = GetDlgItem(Some(h), id) {
            let _ = EnableWindow(ctl, !cfg.pinyin_mode);
        }
    }
    // 辅助码整个关了，「3 键直出」自然无从谈起，同样灰掉
    if let Ok(ctl) = GetDlgItem(Some(h), ID_ASSIST_DIRECT) {
        let _ = EnableWindow(ctl, cfg.assist_mode != AssistMode::Off && !cfg.pinyin_mode);
    }

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
        ID_MODE_DS => config::set_pinyin_mode(false),
        ID_MODE_FULL => config::set_pinyin_mode(true),
        // 启动默认：只改设置，不动当前会话的中英状态——下次激活/开机才生效
        ID_DEF_CN => config::update(|c| c.chinese = true),
        ID_DEF_EN => config::update(|c| c.chinese = false),
        ID_SCHEME_XH => apply_scheme(Scheme::Xiaohe),
        ID_SCHEME_MS => apply_scheme(Scheme::Ms),
        ID_ASSIST_OFF => config::update(|c| c.assist_mode = AssistMode::Off),
        ID_ASSIST_ZRM => config::update(|c| c.assist_mode = AssistMode::Ziranma),
        ID_ASSIST_XH => config::update(|c| c.assist_mode = AssistMode::Xiaohe),
        ID_SKIN_LIGHT => config::update(|c| c.skin = Skin::Light),
        ID_SKIN_DARK => config::update(|c| c.skin = Skin::Dark),
        ID_SKIN_EYE => config::update(|c| c.skin = Skin::Eye),
        ID_HANZI_SIMP => config::update(|c| c.traditional = false),
        ID_HANZI_TRAD => config::update(|c| c.traditional = true),
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
        ID_LEARN => {
            let on = checked(GetDlgItem(Some(h), ID_LEARN).unwrap_or_default());
            config::update(|c| c.learn = on);
        }
        ID_READING => {
            let on = checked(GetDlgItem(Some(h), ID_READING).unwrap_or_default());
            config::set_reading(on);
        }
        ID_ROWS => {
            let sel = GetDlgItem(Some(h), ID_ROWS)
                .ok()
                .and_then(|cb| combo_pick(cb));
            if let Some(i) = sel {
                config::set_candidate_rows(i + config::ROWS_MIN);
            }
        }
        ID_ASSIST_DIRECT => {
            let on = checked(GetDlgItem(Some(h), ID_ASSIST_DIRECT).unwrap_or_default());
            config::update(|c| c.assist_direct = on);
        }
        ID_POPUP_AUTO | ID_POPUP_ALWAYS | ID_POPUP_NEVER => {            let mode = match id {
                ID_POPUP_ALWAYS => PopupMode::Always,
                ID_POPUP_NEVER => PopupMode::Never,
                _ => PopupMode::Auto,
            };
            config::set_popup_mode(mode);
            // 切到「不弹」时立刻把可能挂着的窗口收掉，用户才看得出「生效了」；
            // 切回来时立刻把浮窗放回来，否则要等下一次切换输入法才出现。
            if config::no_popup() {
                crate::candidate::hide();
                statusbar::hide();
            } else {
                statusbar::show();
            }
        }
        ID_LEARN_RESET => {
            let n = crate::freq::clear();
            let m = crate::phrase::clear();
            set_text(
                h,
                ID_LEARN_HINT,
                &format!("已清空 {n} 条词频、{m} 个自造词"),
            );
        }
        ID_PIN_RESET => {
            let n = crate::pin::clear();
            set_text(h, ID_PIN_HINT, &format!("已取消 {n} 个编码的置顶"));
        }
        ID_IMPORT => {
            if let Some(path) = pick_file(h, false) {
                match dict::import_from(&path) {
                    Ok((n, Some((bn, bytes)))) => set_text(
                        h,
                        ID_NOTE,
                        &format!(
                            "已导入 {n} 条新词，并编译成 user.bin（{bn} 条 / {} KB）。",
                            bytes / 1024
                        ),
                    ),
                    Ok((n, None)) => set_text(
                        h,
                        ID_NOTE,
                        &format!("已导入 {n} 条新词（user.bin 编译失败，先用文本模式）。"),
                    ),
                    Err(e) => set_text(h, ID_NOTE, &format!("导入失败：{e}")),
                }
            }
        }
        ID_COMPILE => {
            // 手改过 user_dict.txt、或想确认 user.bin 状态时用。
            // 编译是幂等的：内容没变也会重写一遍，成本只有几十毫秒。
            match dict::compile_user_bin() {
                Ok(dict::CompileOutcome::Compiled { count, bytes }) => set_text(
                    h,
                    ID_NOTE,
                    &format!(
                        "已编译 {count} 条到 user.bin（{} KB）。其它程序重新切换一次输入法后生效。",
                        bytes / 1024
                    ),
                ),
                // 没导入过个人词库：这不是错误，别拿「找不到文件」吓人
                Ok(dict::CompileOutcome::NoSource { existing }) => set_text(
                    h,
                    ID_NOTE,
                    &match existing {
                        Some(n) => format!("没有 user_dict.txt，现有 user.bin {n} 条，无需编译。"),
                        None => "还没有个人词库。先用「导入词库…」导入一个词表。".to_string(),
                    },
                ),
                Err(e) => set_text(h, ID_NOTE, &format!("编译失败：{e}")),
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
    label(h, "输入模式", 16, y, 90, 0);
    radio(h, "双拼", 110, y - 2, 70, ID_MODE_DS, true);
    radio(h, "全拼", 190, y - 2, 70, ID_MODE_FULL, false);

    y += 40;
    label(h, "启动默认", 16, y, 90, 0);
    radio(h, "中文", 110, y - 2, 70, ID_DEF_CN, true);
    radio(h, "英文", 190, y - 2, 70, ID_DEF_EN, false);
    label(
        h,
        "每次激活输入法时进入的模式；Shift 切换只影响当前会话",
        16,
        y + 18,
        428,
        0,
    );

    y += 58; // 上面多了一行说明
    label(h, "双拼方案", 16, y, 90, 0);
    radio(h, "小鹤双拼", 110, y - 2, 100, ID_SCHEME_XH, true);
    radio(h, "微软双拼", 220, y - 2, 100, ID_SCHEME_MS, false);

    y += 40;
    label(h, "辅助码", 16, y, 90, 0);
    radio(h, "关闭", 110, y - 2, 70, ID_ASSIST_OFF, true);
    radio(h, "自然码辅助码", 190, y - 2, 120, ID_ASSIST_ZRM, false);
    radio(h, "小鹤音形", 320, y - 2, 100, ID_ASSIST_XH, false);
    // 辅助码必须按 Tab 才生效，这一步没写出来就没人知道（用户真的是打完了才想起）
    label(
        h,
        "Tab 进出；自然码 1 键 / 小鹤 2 键，先打完码再按 Tab 也认",
        16,
        y + 18,
        428,
        0,
    );
    // 「3 键直出」紧跟在辅助码说明下面：它只改「第 3 键算谁的」，是同一件事的变体
    check(
        h,
        "3 键直出单字（第 3 键直接当形码，如 ddj → 带）",
        16,
        y + 38,
        428,
        ID_ASSIST_DIRECT,
    );

    y += 80; // 上面多了一行说明 + 一行勾选
    label(h, "皮肤", 16, y, 90, 0);
    radio(h, "浅色", 110, y - 2, 70, ID_SKIN_LIGHT, true);
    radio(h, "深色", 190, y - 2, 70, ID_SKIN_DARK, false);
    radio(h, "护眼", 270, y - 2, 70, ID_SKIN_EYE, false);

    y += 40;
    label(h, "字形", 16, y, 90, 0);
    radio(h, "简体", 110, y - 2, 70, ID_HANZI_SIMP, true);
    radio(h, "繁体", 190, y - 2, 70, ID_HANZI_TRAD, false);

    y += 40;
    check(h, "全角标点（，。、；等）", 16, y - 2, 220, ID_FULLWIDTH);

    y += 30;
    check(h, "状态浮窗跟随光标（默认固定在屏幕右下角，可拖动）", 16, y - 2, 420, ID_FOLLOW);

    y += 30;
    label(h, "候选窗", 16, y, 90, 0);
    radio(h, "自动", 110, y - 2, 70, ID_POPUP_AUTO, true);
    radio(h, "始终弹出", 190, y - 2, 100, ID_POPUP_ALWAYS, false);
    radio(h, "始终不弹", 300, y - 2, 100, ID_POPUP_NEVER, false);
    label(
        h,
        "自动：全屏游戏 / 演示 / 全屏应用时，候选写进组合串、不弹自绘窗口",
        16,
        y + 18,
        428,
        0,
    );
    label(
        h,
        "「始终不弹」会把浮窗一起收起，此时用 Ctrl+Shift+U 临时切回",
        16,
        y + 36,
        428,
        0,
    );
    y += 38; // 给上面两行提示留位置

    // 读音提示紧跟在候选窗设置后面：两者都是「候选长什么样」
    y += 6;
    check(
        h,
        "易错词标注读音（重庆 chóng qìng）",
        16,
        y - 2,
        420,
        ID_READING,
    );
    y += 26;

    // 候选窗摊开几行：多行找字比一页 9 个快得多，但又不能替用户决定，
    // 所以做成下拉——1 行就是老样子，谁都不被迫改变习惯。
    label(h, "翻页时显示行数", 16, y, 110, 0);
    let cb = combo(h, 130, y - 4, 60, ID_ROWS);
    for n in config::ROWS_MIN..=config::ROWS_MAX {
        combo_add(cb, &n.to_string());
    }
    label(h, "行", 196, y, 20, 0);
    label(
        h,
        "按 + / = 进翻页时摊开成多行，回到第一页再按 - / ↑ 收起",
        216,
        y,
        226,
        0,
    );
    y += 26;

    y += 12;
    check(h, "动态调频（按实际选词调整候选顺序）", 16, y - 2, 300, ID_LEARN);
    push(h, "重置", 320, y - 5, 70, 26, ID_LEARN_RESET);
    y += 26;
    label(h, "", 16, y, 428, ID_LEARN_HINT);

    // 置顶：和一简字一样属于「候选顺序」这件事，紧跟在动态调频下面
    y += 26;
    label(h, "", 16, y, 300, ID_PIN_HINT);
    push(h, "清空", 320, y - 5, 70, 26, ID_PIN_RESET);

    y += 40;
    label(h, "", 16, y, 428, ID_STATS);

    // 数据目录：备份时要压的就是这个文件夹
    y += 22;
    label(h, "", 16, y, 428, ID_DATA_DIR);

    y += 34;
    push(h, "导入词库…", 16, y, 104, 28, ID_IMPORT);
    push(h, "导出词库…", 128, y, 104, 28, ID_EXPORT);
    push(h, "编译个人词库", 240, y, 128, 28, ID_COMPILE);

    y += 38;
    label(h, "导入格式：每行「词<TAB>全拼」，例如：你好<TAB>ni hao", 16, y, 428, ID_NOTE);

    // 快捷输入说明。码表从 quick 模块取，避免文档和实现对不上。
    // 八个命令一行放不下（428px 会截断），按四个一行折开。
    let quick: Vec<String> = crate::quick::commands()
        .iter()
        .map(|(k, name)| format!("{}{k} {name}", crate::quick::PREFIX))
        .collect();
    let rows = quick.chunks(4).count() as i32;
    for (i, chunk) in quick.chunks(4).enumerate() {
        let text = if i == 0 {
            format!("快捷输入：{}", chunk.join(" · "))
        } else {
            format!("　　　　　{}", chunk.join(" · "))
        };
        label(h, &text, 16, y + 26 + i as i32 * 20, 428, 0);
    }

    // `、` `|` 和翻页键是最容易被问到的几个键，直接写在设置里省得去翻文档
    label(
        h,
        "标点：全角时 \\ 出「、」，Shift+\\ 出「|」 · 翻页：- = ↑ ↓",
        16,
        y + 26 + rows * 20 + 4,
        428,
        0,
    );

    push(h, "关闭", W - 106, CLIENT_H - 40, 90, 30, ID_CLOSE);
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
            // 只处理点击（普通控件）和下拉框的「换了一项」；忽略其它通知噪音
            if code == 0 || code == CBN_SELCHANGE {
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

/// 窗口样式（创建和反算尺寸必须用同一份，否则算出来的高度对不上）
const STYLE: WINDOW_STYLE = WINDOW_STYLE(
    WS_OVERLAPPED.0 | WS_CAPTION.0 | WS_SYSMENU.0 | WS_MINIMIZEBOX.0,
);

/// 由客户区尺寸反算窗口尺寸。标题栏和边框高度随 DPI / 主题变，写死窗口高度
/// 会在高 DPI 下把最下面那排控件截掉。
unsafe fn window_size(client_w: i32, client_h: i32) -> (i32, i32) {
    let mut rc = RECT {
        left: 0,
        top: 0,
        right: client_w,
        bottom: client_h,
    };
    if AdjustWindowRectEx(&mut rc, STYLE, false, WINDOW_EX_STYLE(0)).is_err() {
        // 反算失败就按 1:1 用，宁可挤一点也不要开不出窗口
        return (client_w, client_h);
    }
    (rc.right - rc.left, rc.bottom - rc.top)
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

    let (win_w, win_h) = window_size(W, CLIENT_H);
    // 标题里带版本号：用户报问题、我们查「是哪一版」全靠这一行。手写常量迟早和
    // Cargo.toml 对不上（以前就吃过「用户装的是 0.2.2、代码是 0.2.5」的亏），
    // 所以直接从编译期包版本取，改版本号不用再回来改这里。
    let title = format!("飞鹤输入法 · 设置 · {}", env!("CARGO_PKG_VERSION"));
    let title_w = to_wide(&title);
    let h = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        CLASS_NAME,
        PCWSTR(title_w.as_ptr()),
        STYLE,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        win_w,
        win_h,
        None,
        None,
        Some(HINSTANCE(hinst.0)),
        None,
    )
    .unwrap_or_default();
    if h.is_invalid() {
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

pub fn destroy() {
    let raw = WND.swap(0, Ordering::AcqRel);
    if raw == 0 {
        return;
    }
    unsafe {
        let _ = DestroyWindow(HWND(raw as *mut core::ffi::c_void));
    }
}
