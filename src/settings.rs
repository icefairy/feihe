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

use std::sync::atomic::{AtomicI32, AtomicIsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{ScreenToClient, COLOR_WINDOW, HBRUSH};
use windows::Win32::System::LibraryLoader::{
    GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};
// SetScrollInfo 在 windows-rs 里归在 Controls 下（GetScrollInfo 还在 WindowsAndMessaging）
// 滚动时的原子重画：RedrawWindow 和 RDW_* 标志归 Gdi
use windows::Win32::Graphics::Gdi::{
    RedrawWindow, RDW_ALLCHILDREN, RDW_ERASE, RDW_FRAME, RDW_INVALIDATE, RDW_UPDATENOW,
};
use windows::Win32::UI::Controls::SetScrollInfo;
use windows::Win32::UI::Controls::Dialogs::*;
// EnableWindow 在 windows-rs 里归到了 KeyboardAndMouse 模块（跟着 user32 的分组走）
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
// 打开项目主页走 shell（由系统的「默认浏览器」决定，不是我们起进程）
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::assist::AssistMode;
use crate::config::{self, PopupMode, Skin};
use crate::dict;
use crate::dpi;
use crate::font;
use crate::shuangpin::Scheme;
use crate::statusbar;

const CLASS_NAME: PCWSTR = w!("FeiheSettings");

/// **客户区**尺寸，单位是**逻辑像素**（96 DPI 下的样子）。
///
/// 真正下发给系统的尺寸一律过 [`px`]：本工程是进程内 COM 组件，没法把宿主改成
/// DPI 感知，只能问出当前 DPI 再按比例放大自己的窗口。不这么做的话，150% 缩放的
/// 屏上设置窗只有 460 物理像素宽——同一行里挤着三列单选按钮，文字被截成半个字。
///
/// 窗口尺寸仍由 `window_size()` 反算——标题栏高度随 DPI 和系统主题变，
/// 写死窗口高度会在高 DPI / 大字号下把最下面那排按钮截掉。
const W: i32 = 460;

/// 客户区高度上限。**内容超出就滚动**，不再靠加高窗口硬塞——
/// 设置项只会越来越多，而 1366×768 的笔记本（国内存量很大）连 900 都装不下。
const CLIENT_H_MAX: i32 = 820;

/// 客户区高度下限：再矮就只剩两三个设置项，滚动条也基本没法用。
const CLIENT_H_MIN: i32 = 460;

/// 底部「关闭」那一栏固定不动，滚内容时不跟着走。这是给它留的高度。
const FOOTER_H: i32 = 52;

/// 把逻辑像素换算成当前 DPI 下的物理像素。
///
/// 设置窗里所有常量、`build()` 里所有坐标都是**逻辑值**；只有跟系统对话
/// （`GetSystemMetrics`、客户区尺寸、`CreateWindowExW`）时才需要物理值，
/// 所以换算集中在 [`mk`] / [`window_size`] / [`client_h`] / [`want_client_h`]
/// 这几个边界上做，布局代码里一个 `px()` 都不用写——否则每个 `y += 40` 后面
/// 都要跟一句换算，漏一个就在某个缩放下错位。
fn px(v: i32) -> i32 {
    dpi::px(v)
}

/// 项目主页。设置窗最下面显示它，点按钮直接用默认浏览器打开。
///
/// 写在代码里而不是读某个配置：地址是产品身份的一部分，改了要发版，不该被用户
/// 或者一次失败的写盘悄悄改掉。
const ABOUT_URL: &str = "https://github.com/icefairy/feihe";

/// 上一次「从搜狗导入」的时间，配合 `IMPORT_DEBOUNCE` 给按钮双击去抖。
static LAST_IMPORT: Mutex<Option<Instant>> = Mutex::new(None);
/// 去抖窗口。导入一次几百毫秒，正常人不会在这个间隔内再点一次。
const IMPORT_DEBOUNCE: Duration = Duration::from_secs(5);

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
/// 打开 `custom.txt`（自定义短语）。
const ID_CUSTOM: i32 = 190;

/// 玄学分类词库开关（干支/易经/奇门……，默认关）。
const ID_XUANXUE: i32 = 191;

/// 模糊音九条规则的复选框，ID 连续：`ID_FUZZY_FIRST + 序号`，序号就是
/// `fuzzy::NAMES` 的下标。加减规则时只改那张表，不用再回到这里加常量。
const ID_FUZZY_FIRST: i32 = 181;
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
/// 降权（`Ctrl`+左键点候选）
const ID_DEMOTE_HINT: i32 = 173;
const ID_DEMOTE_RESET: i32 = 174;
const ID_SUGGEST: i32 = 146;
const ID_SUGGEST_HINT: i32 = 176;
const ID_SUGGEST_RESET: i32 = 175;
const ID_DATA_DIR: i32 = 145;
const ID_IMPORT: i32 = 151;
const ID_EXPORT: i32 = 152;
const ID_CLOSE: i32 = 153;
const ID_COMPILE: i32 = 154;
const ID_SOGOU: i32 = 157;
const ID_SOGOU_FILE: i32 = 158;
const ID_ABOUT: i32 = 155;
const ID_ABOUT_LINK: i32 = 156;

static WND: AtomicIsize = AtomicIsize::new(0);

/// 当前滚动偏移（客户区像素，0 = 顶部）。
static SCROLL: AtomicI32 = AtomicI32::new(0);

/// 内容总高度（`build` 结束时算出来，滚动范围靠它）。
static CONTENT_H: AtomicI32 = AtomicI32::new(0);

/// 可以滚动的子控件：`(HWND, 创建时的 x, 创建时的 y)`。滚动时按「基准 - 偏移」重摆，
/// 不重新算布局——布局只有 `build` 一处，重复实现迟早对不上。
static SCROLLABLE: Mutex<Vec<(isize, i32, i32)>> = Mutex::new(Vec::new());

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
    // 布局代码（build() 里那一串 y += 40）全按逻辑值写，物理换算只在这一处做。
    // 反过来（布局里逐处调 px）会漏，漏一个就在某个缩放下错位，而且很难看出来。
    let child = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        class,
        PCWSTR(wide.as_ptr()),
        WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0) | style,
        px(x),
        px(y),
        px(w),
        px(h),
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

/// 看起来像文字的「链接」：静态控件加 `SS_NOTIFY`，点一下会发一条 `STN_CLICKED`
/// 到父窗口（`WM_COMMAND` 的 code 正好是 0，和普通按钮点击走同一条路）。
///
/// 不用 SysLink 是因为它默认带下划线和蓝色、和这里「一行灰色小字」的排版不搭；
/// 也不用自绘，设置窗整体走原生控件。
fn link_label(parent: HWND, text: &str, x: i32, y: i32, w: i32, id: i32) -> HWND {
    unsafe {
        mk(
            parent,
            w!("STATIC"),
            text,
            // SS_NOTIFY = 0x100：静态控件默认「点不动」，只发 STN_CLICKED 才需要它。
            // windows-rs 0.62 没有导出这个常量（连 WindowsAndMessaging 里都没有），
            // 所以写数值——这是静态控件的稳定 ABI，不会变。
            WINDOW_STYLE(0x0000_0100),
            x,
            y,
            w,
            20,
            id,
        )
    }
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
    set_checked(h, ID_XUANXUE, cfg.xuanxue);
    set_checked(h, ID_ASSIST_DIRECT, cfg.assist_direct);
    for (i, name) in crate::fuzzy::NAMES.iter().enumerate() {
        set_checked(h, ID_FUZZY_FIRST + i as i32, cfg.fuzzy.is_on(name));
    }
    set_checked(h, ID_LEARN, cfg.learn);
    set_checked(h, ID_SUGGEST, cfg.suggest);
    // 「联想」的数据量级和「置顶/降权」一样小，直接把条数写出来，用户能判断
    // 它到底学没学到东西。内置表（发货就带的）单独列——清空只能清掉前者，
    // 不写清楚的话「清空完还能联想」看起来就像没生效。
    set_text(
        h,
        ID_SUGGEST_HINT,
        &format!(
            "已记 {} 个词对（另内置 {} 条）· 只学你自己的输入",
            crate::next::count(),
            crate::next::builtin_count()
        ),
    );
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
        0 => "置顶：无 · 右键点候选 或 Ctrl+数字".to_string(),
        n if n <= 2 => format!(
            "置顶 {} · 右键点候选切换",
            pinned
                .iter()
                .map(|(c, w)| format!("{c}→{w}"))
                .collect::<Vec<_>>()
                .join(" · ")
        ),
        n => format!("置顶 {n} 个编码 · 右键点候选切换"),
    };
    set_text(h, ID_PIN_HINT, &pin_text);

    // 降权是「把一个词按下去」，和置顶（拉一个上来）是一对，紧跟在它下面。
    // 入口写在文案里——这个功能不占快捷键，用户没法自己摸出来。
    let demoted = crate::demote::count();
    set_text(
        h,
        ID_DEMOTE_HINT,
        &format!("降权 {demoted} 条 · Ctrl+左键点候选"),
    );

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
            "词库：共 {total} 条，其中用户词 {user} 条（{}）· 当前编码方案：{}",
            dict::user_source_label(),
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
    pick_file_filtered(
        h,
        save,
        "词库文件 (*.txt)\0*.txt\0所有文件 (*.*)\0*.*\0",
        if save { "导出词库" } else { "导入词库" },
        "txt",
    )
}

/// 细胞词库文件选择框：从搜狗词库网站下载的就是 .scel，放在哪个目录都行，
/// 跟本机装没装搜狗无关。
fn pick_scel_file(h: HWND) -> Option<std::path::PathBuf> {
    pick_file_filtered(
        h,
        false,
        "搜狗细胞词库 (*.scel)\0*.scel\0所有文件 (*.*)\0*.*\0",
        "导入细胞词库",
        "scel",
    )
}

fn pick_file_filtered(
    h: HWND,
    save: bool,
    filter_text: &str,
    title: &str,
    defext: &str,
) -> Option<std::path::PathBuf> {
    let mut buf = [0u16; 1024];
    // OFN 的 filter 以「描述\0模式\0」成对出现，结尾要多一个 \0 收口
    let filter: Vec<u16> = format!("{filter_text}\0").encode_utf16().collect();
    let title_w = to_wide(title);
    let defext = to_wide(defext);

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
        ID_XUANXUE => {
            let on = checked(GetDlgItem(Some(h), ID_XUANXUE).unwrap_or_default());
            config::set_xuanxue(on);
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
            // 必须 return：函数尾部有个 `sync(h)`，会把这两行重写成统计文案
            // （「已记 0 个词频」），确认消息在同一次 WM_COMMAND 里被覆盖掉，
            // 用户永远看不到「已清空」。
            set_text(
                h,
                ID_LEARN_HINT,
                &format!("已清空 {n} 条词频、{m} 个自造词"),
            );
            return;
        }
        ID_PIN_RESET => {
            let n = crate::pin::clear();
            set_text(h, ID_PIN_HINT, &format!("已取消 {n} 个编码的置顶"));
            return;
        }
        ID_SUGGEST => {
            let on = checked(GetDlgItem(Some(h), ID_SUGGEST).unwrap_or_default());
            config::update(|c| c.suggest = on);
            // 关掉之后表还在，只是不再显示——重新打开时立刻就有词，不用重新学。
            // 想真清空是右边那个「清空」按钮的事，两件事分开。
        }
        ID_SUGGEST_RESET => {
            let n = crate::next::count();
            crate::next::clear();
            set_text(h, ID_SUGGEST_HINT, &format!("已清空 {n} 个词对"));
            return;
        }
        ID_DEMOTE_RESET => {
            let n = crate::demote::clear();
            set_text(h, ID_DEMOTE_HINT, &format!("已恢复 {n} 条被降权的候选"));
            return;
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
        ID_SOGOU => {
            // 双击去抖。按钮双击会连来两条 `WM_COMMAND`（双击 = 两次
            // BN_CLICKED），而导入是在窗口过程里**同步**跑的，两条都会完整执行
            // 一遍：界面冻结两次，第二条必然报「没有新词——都已经在个人词库里
            // 了」，用户以为功能坏了。导入期间禁用按钮挡不住已经排队的消息
            // （那会儿窗口过程还压在导入里，根本来不及禁用），只能按时间去抖。
            {
                let mut g = LAST_IMPORT.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(t) = *g {
                    if t.elapsed() < IMPORT_DEBOUNCE {
                        return;
                    }
                }
                *g = Some(Instant::now());
            }
            // 解析几个 MB 的 bin + 编译 user.bin 大概几百毫秒，同步做就行。
            // 但必须换沙漏光标：不然用户会以为按钮没反应，连点好几下。
            let r = with_wait_cursor(|| crate::sogou::import_auto(crate::sogou::MAX_LEN));
            match r {
                Ok(s) => set_text(h, ID_NOTE, &sogou_summary(&s)),
                Err(e) => {
                    // 自动发现失败（装在别的盘 / 用的是绿色版）就让他手选目录。
                    // 不直接弹文件夹选择框，是为了让「为什么没找到」这句先说清楚。
                    if ask_yes_no(h, "从搜狗导入", &format!("{e}\n\n要手动选择搜狗的用户数据文件夹吗？"))
                    {
                        if let Some(dir) = pick_folder(h) {
                            let r = with_wait_cursor(|| {
                                crate::sogou::import_dir(&dir, crate::sogou::MAX_LEN)
                            });
                            match r {
                                Ok(s) => set_text(h, ID_NOTE, &sogou_summary(&s)),
                                Err(e) => set_text(h, ID_NOTE, &format!("导入失败：{e}")),
                            }
                        }
                    } else {
                        set_text(h, ID_NOTE, "未导入（没找到搜狗词库）");
                    }
                }
            }
        }
        ID_SOGOU_FILE => {
            // 从搜狗词库网站下载的细胞词库是单个 .scel 文件，放在哪个目录都行，
            // 跟本机装没装搜狗无关——所以给独立的选文件入口，而不是塞进
            // 「从搜狗导入」的自动发现/选目录流程里让用户绕。
            if let Some(path) = pick_scel_file(h) {
                let r = with_wait_cursor(|| {
                    crate::sogou::import_file(&path, crate::sogou::MAX_LEN)
                });
                match r {
                    Ok(s) => set_text(h, ID_NOTE, &sogou_summary(&s)),
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
        // 项目主页：交给默认浏览器，自己不联网
        ID_ABOUT | ID_ABOUT_LINK => open_about(),
        // 自定义短语：交给系统默认的编辑器（.txt → 记事本），我们不内建编辑器
        ID_CUSTOM => open_custom(),
        // 模糊音九条：ID 连续，按 `NAMES` 的下标一次派发完
        id if (ID_FUZZY_FIRST..ID_FUZZY_FIRST + crate::fuzzy::NAMES.len() as i32).contains(&id) => {
            let name = crate::fuzzy::NAMES[(id - ID_FUZZY_FIRST) as usize];
            // 是 `BS_AUTOCHECKBOX`，点完系统已经把勾翻过了，按当前状态写配置
            let on = checked(GetDlgItem(Some(h), id).unwrap_or_default());
            config::update(|c| {
                c.fuzzy.set(name, on);
            });
        }
        _ => return,
    }

    // 皮肤 / 跟随 变更要立刻反映到浮窗和候选窗
    statusbar::refresh();
    crate::candidate::refresh_theme();
    sync(h);
}

/// 用默认浏览器打开项目主页。
///
/// 走 shell 而不是自己 `CreateProcess`：用哪个浏览器、是不是要提权，全由系统决定；
/// 也不校验成功与否——设置窗不该因为浏览器打不开而弹错误框。
unsafe fn open_about() {
    let op = to_wide("open");
    let url = to_wide(ABOUT_URL);
    let _ = ShellExecuteW(
        None,
        PCWSTR(op.as_ptr()),
        PCWSTR(url.as_ptr()),
        PCWSTR::null(),
        PCWSTR::null(),
        SW_SHOWNORMAL,
    );
}

/// 打开 `custom.txt` 给用户编辑。
///
/// 文件不存在时先写一份带注释的模板：空文件看不出格式，用户还得回来翻文档。
/// 模板只在**点这个按钮**时写——启动时就写的话，用户删掉文件想清空，一重启又长回来了。
unsafe fn open_custom() {
    crate::custom::ensure_template();
    let op = to_wide("open");
    let path = to_wide(&crate::custom::file().to_string_lossy());
    let _ = ShellExecuteW(
        None,
        PCWSTR(op.as_ptr()),
        PCWSTR(path.as_ptr()),
        PCWSTR::null(),
        PCWSTR::null(),
        SW_SHOWNORMAL,
    );
}

/// 一句人话把导入结果说完。用户只关心「多了多少词」，其余是排查用的。
///
/// **文案必须短**：`ID_NOTE` 那条静态标签只有 428px，13px 字体下一行约 32 个
/// 全角字，超长的部分直接被裁掉——用户看到的是半句话。数字和汉字之间不加空格
/// 也是为这个；更细的数字（扫了多少条、多少走基础词库、多少逐字注音）都在
/// `feihe.log` 里。
fn sogou_summary(s: &crate::sogou::Imported) -> String {
    if s.added == 0 {
        return format!(
            "已解析{}个文件（{}条），没有新词——都已在词库里。",
            s.files, s.encoded
        );
    }
    format!(
        "已导入{}个新词（{}个文件，跳过{}条）。切换输入法后生效。",
        s.added, s.files, s.skipped
    )
}

/// 执行期间把光标换成沙漏。
///
/// 不用 `SetCursor` 的返回值恢复：沙漏只是「等一会儿」的提示，漏恢复一次
/// （比如中途 return）就一直转圈，比不换还糟。所以统一在这里成对处理。
fn with_wait_cursor<T>(f: impl FnOnce() -> T) -> T {
    unsafe {
        let wait = LoadCursorW(None, IDC_WAIT).unwrap_or_default();
        let _ = SetCursor(Some(wait));
    }
    let out = f();
    unsafe {
        let arrow = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        let _ = SetCursor(Some(arrow));
    }
    out
}

unsafe fn ask_yes_no(h: HWND, title: &str, text: &str) -> bool {
    // 两串必须绑到局部变量：`PCWSTR(to_wide(x).as_ptr())` 写在同一行也能跑
    // （临时值活到语句结束），但那是在赌生命周期规则，后面谁「整理一下」就成了
    // 悬垂指针。绑出来一眼看得懂。
    let text = to_wide(text);
    let title = to_wide(title);
    MessageBoxW(
        Some(h),
        PCWSTR(text.as_ptr()),
        PCWSTR(title.as_ptr()),
        MB_YESNO | MB_ICONQUESTION,
    ) == IDYES
}

/// 选一个文件夹（`SHBrowseForFolderW`）。
///
/// 为什么不用 `IFileOpenDialog` + `FOS_PICKFOLDERS`：那要走 COM 初始化，
/// 而我们可能在前台宿主的 STA 里，`CoInitializeEx` 返回 RPC_E_CHANGED_MODE
/// 的边界要处理一堆。老 API 丑但没这些坑。
fn pick_folder(h: HWND) -> Option<std::path::PathBuf> {
    use windows::Win32::UI::Shell::{
        SHBrowseForFolderW, SHGetPathFromIDListW, BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS,
        BROWSEINFOW,
    };

    let title = to_wide("选择搜狗的用户数据文件夹（里面应有 sgim_gd_usrbg.bin）");
    let bi = BROWSEINFOW {
        hwndOwner: h,
        lpszTitle: PCWSTR(title.as_ptr()),
        ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
        ..Default::default()
    };
    unsafe {
        let pidl = SHBrowseForFolderW(&bi);
        if pidl.is_null() {
            return None;
        }
        // 这个函数只接受 MAX_PATH（260）的定长缓冲，不是随便一个切片
        let mut buf = [0u16; 260];
        let ok = SHGetPathFromIDListW(pidl, &mut buf).as_bool();
        windows::Win32::System::Com::CoTaskMemFree(Some(pidl as *const core::ffi::c_void));
        if !ok {
            return None;
        }
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(std::path::PathBuf::from(String::from_utf16_lossy(&buf[..n])))
    }
}

/// 这台机器上设置窗该**多高**（逻辑像素）：屏幕矮的时候按屏幕缩
/// （1366×768 的笔记本很常见），但不低于 CLIENT_H_MIN、不超过 CLIENT_H_MAX。
///
/// `GetSystemMetrics(SM_CYSCREEN)` 给的是**物理**像素，必须先换回逻辑值再夹范围：
/// 150% 缩放的 768p 屏物理高 768、逻辑高 512，直接拿 768 去夹会以为屏幕很宽裕，
/// 结果窗口高过屏幕、底部按钮跑到屏幕外。这和「1366×768 装不下 900 客户区」
/// 是同一个老问题，只是高 DPI 下换了副面孔回来。
unsafe fn want_client_h() -> i32 {
    let screen = dpi::unscale(GetSystemMetrics(SM_CYSCREEN), dpi::dpi());
    (screen - 120).clamp(CLIENT_H_MIN, CLIENT_H_MAX)
}

/// 当前客户区高度，**物理**像素。
///
/// 窗口建好之后以实际值为准——高 DPI 下按比例换算可能差几个像素，而底部按钮
/// 必须按真实高度摆。滚动相关的量（CONTENT_H / 滚动位置 / SetWindowPos）
/// 也统一留在这个坐标系里，逻辑坐标系只在布局代码里用。
unsafe fn client_h(h: HWND) -> i32 {
    let mut rc = RECT::default();
    if GetClientRect(h, &mut rc).is_err() {
        return px(CLIENT_H_MAX);
    }
    (rc.bottom - rc.top).max(px(CLIENT_H_MIN))
}

/// 可滚动区域高度：客户区减去底部那条固定栏。两者都是物理值。
unsafe fn view_h(h: HWND) -> i32 {
    (client_h(h) - px(FOOTER_H)).max(1)
}

/// 记下每个子控件创建时的位置，供滚动时重摆。
///
/// 排除「关闭」按钮：它固定在底部，滚内容时不能跟着走——滚到一半找不到关闭按钮
/// 是很恼人的。其余控件全部参与滚动。
unsafe fn init_scroll(h: HWND) {
    let footer = GetDlgItem(Some(h), ID_CLOSE).unwrap_or_default();
    let mut list: Vec<(isize, i32, i32)> = Vec::new();
    let mut child = GetWindow(h, GW_CHILD).unwrap_or_default();
    while !child.is_invalid() {
        if child != footer {
            let mut rc = RECT::default();
            if GetWindowRect(child, &mut rc).is_ok() {
                let mut p = POINT {
                    x: rc.left,
                    y: rc.top,
                };
                if ScreenToClient(h, &mut p).as_bool() {
                    list.push((child.0 as isize, p.x, p.y));
                }
            }
        }
        child = GetWindow(child, GW_HWNDNEXT).unwrap_or_default();
    }
    if let Ok(mut g) = SCROLLABLE.lock() {
        *g = list;
    }
    SCROLL.store(0, Ordering::Release);
    sync_scrollbar(h);
}

/// 按内容高度和可视高度把滚动条范围刷一遍。
unsafe fn sync_scrollbar(h: HWND) {
    let view = view_h(h);
    let content = CONTENT_H.load(Ordering::Acquire);
    let max_pos = (content - view).max(0);
    let pos = SCROLL.load(Ordering::Acquire).clamp(0, max_pos);
    let si = SCROLLINFO {
        cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
        fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
        nMin: 0,
        nMax: (content - 1).max(0),
        nPage: view as u32,
        nPos: pos,
        nTrackPos: pos,
    };
    SetScrollInfo(h, SB_VERT, &si, true);
}

/// 滚到 `pos`：夹进合法范围，再按「基准位置 - 偏移」重摆所有可滚控件。
unsafe fn scroll_to(h: HWND, pos: i32) {
    let max_pos = (CONTENT_H.load(Ordering::Acquire) - view_h(h)).max(0);
    let pos = pos.clamp(0, max_pos);
    let cur = SCROLL.load(Ordering::Acquire);
    if pos == cur {
        // 位置没变也要刷一次：内容高度可能在这次 build 里变过
        sync_scrollbar(h);
        return;
    }
    SCROLL.store(pos, Ordering::Release);
    if let Ok(g) = SCROLLABLE.lock() {
        // 残影的根因：逐个 SetWindowPos 会让每个子窗口各自异步重画，而父窗口
        // 暴露出来的空隙没人擦——先动的画完了、后动的还没挪，中间态就留在屏幕上。
        // 对策三步：挂起重绘 → 全部挪完 → 原子重画。UpdateWindow 只管父窗口自己，
        // 子窗口的过期区域得过 RDW_ALLCHILDREN 才能一起同步刷掉。
        let _ = SendMessageW(h, WM_SETREDRAW, Some(WPARAM(0)), Some(LPARAM(0)));
        for &(hwnd, x, base) in g.iter() {
            let child = HWND(hwnd as *mut core::ffi::c_void);
            let _ = SetWindowPos(
                child,
                None,
                x,
                base - pos,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        let _ = SendMessageW(h, WM_SETREDRAW, Some(WPARAM(1)), Some(LPARAM(0)));
        let _ = RedrawWindow(
            Some(h),
            None,
            None,
            RDW_INVALIDATE | RDW_ERASE | RDW_FRAME | RDW_ALLCHILDREN | RDW_UPDATENOW,
        );
    }
    sync_scrollbar(h);
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

    // 模糊音：按口音放宽声母/韵母的对应。九条规则横排成 3×3，一眼看全，
    // 也不用为了"哪个口音"再套一层预设——用户自己知道该勾哪个。
    y += 80; // 上面多了一行说明 + 一行勾选
    label(h, "模糊音", 16, y, 90, 0);
    label(h, "（按口音选，不开就是原样）", 110, y, 330, 0);
    for (i, name) in crate::fuzzy::NAMES.iter().enumerate() {
        let col = (i % 3) as i32;
        let row = (i / 3) as i32;
        check(
            h,
            &crate::fuzzy::Rules::label(name),
            16 + col * 144,
            y + 24 + row * 26,
            140,
            ID_FUZZY_FIRST + i as i32,
        );
    }
    // 三行复选框：首行下移 24、行距 26、控件本身 22 高，再留 8 的段间距
    y += 24 + 2 * 26 + 22 + 8;
    label(h, "皮肤", 16, y, 90, 0);
    radio(h, "浅色", 110, y - 2, 70, ID_SKIN_LIGHT, true);
    radio(h, "深色", 190, y - 2, 70, ID_SKIN_DARK, false);
    radio(h, "护眼", 270, y - 2, 70, ID_SKIN_EYE, false);

    y += 40;
    label(h, "字形", 16, y, 90, 0);
    radio(h, "简体", 110, y - 2, 70, ID_HANZI_SIMP, true);
    radio(h, "繁体", 190, y - 2, 70, ID_HANZI_TRAD, false);

    y += 40;
    check(
        h,
        "全角符号（英文态下标点占一个汉字宽：，．；／；中文态恒为中文标点）",
        16,
        y - 2,
        428,
        ID_FULLWIDTH,
    );

    y += 30;
    check(h, "状态浮窗跟随光标（默认固定在屏幕右下角，可拖动）", 16, y - 2, 420, ID_FOLLOW);

    y += 30;
    label(h, "候选窗", 16, y, 90, 0);
    radio(h, "自动", 110, y - 2, 70, ID_POPUP_AUTO, true);
    radio(h, "始终弹出", 190, y - 2, 100, ID_POPUP_ALWAYS, false);
    radio(h, "始终不弹", 300, y - 2, 100, ID_POPUP_NEVER, false);
    label(
        h,
        "自动：投影 / 演示时收起；全屏游戏照常弹候选窗（只收浮窗）",
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

    // 玄学词库：和“读音提示”同一类——都是“候选里多不多出这批字”。
    //
    // 开关而不是分类列表：这摊词只有一类用户（算命、中医爱好者），
    // 拆成十来个勾选框只会让普通用户不知道该不该勾。
    check(
        h,
        "启用玄学词库（干支、易经、奇门、六壬、六爻、神煞）",
        16,
        y - 2,
        420,
        ID_XUANXUE,
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

    // 联想：同样是「越用越顺手」，紧跟在调频下面。
    // 文案留白要够：勾选框宽 300，右边的「清空」占 320..390，压上去就点错了。
    y += 26;
    check(h, "上屏后联想下一个词", 16, y - 2, 300, ID_SUGGEST);
    push(h, "清空", 320, y - 5, 70, 26, ID_SUGGEST_RESET);
    y += 26;
    label(h, "", 16, y, 428, ID_SUGGEST_HINT);

    // 置顶：和一简字一样属于「候选顺序」这件事，紧跟在动态调频下面
    y += 26;
    label(h, "", 16, y, 300, ID_PIN_HINT);
    push(h, "清空", 320, y - 5, 70, 26, ID_PIN_RESET);

    // 降权：置顶的反面（把一个词按下去），成对出现用户才好理解。
    // 间距必须 >= 26：两个「清空」按钮在同一列，26px 高，压到 24 就会叠 2px。
    y += 26;
    label(h, "", 16, y, 300, ID_DEMOTE_HINT);
    push(h, "清空", 320, y - 5, 70, 26, ID_DEMOTE_RESET);

    y += 40;
    label(h, "", 16, y, 428, ID_STATS);

    // 数据目录：备份时要压的就是这个文件夹
    y += 22;
    label(h, "", 16, y, 428, ID_DATA_DIR);

    y += 34;
    // 四个按钮挤一行：428px 里 96+116+96+96 加上 8px 间隔正好铺满。
    // 「从搜狗导入」自动发现本机搜狗；「细胞词库」直接选下载来的 .scel 文件
    // （搜狗词库网站下载的，跟装没装搜狗无关）——同一个「词从哪来」的家族。
    push(h, "导入词库…", 16, y, 96, 28, ID_IMPORT);
    push(h, "从搜狗导入…", 120, y, 116, 28, ID_SOGOU);
    push(h, "细胞词库…", 244, y, 96, 28, ID_SOGOU_FILE);
    push(h, "导出词库…", 348, y, 96, 28, ID_EXPORT);    y += 38;
    push(h, "编译个人词库", 16, y, 98, 28, ID_COMPILE);
    label(
        h,
        "格式：词<TAB>全拼（你好<TAB>ni hao）· 细胞词库选 .scel，自动注音",
        122,
        y + 6,
        322,
        ID_NOTE,
    );    // 自定义短语：和「导入词库」一样属于「我能喂给它什么」，紧跟在这一块后面。
    // 入口做成按钮而不是写个路径：用户不会为了一个没见过的文件去翻设置目录。
    push(h, "编辑自定义短语…", 16, y + 38, 130, 26, ID_CUSTOM);
    label(
        h,
        "码<TAB>文本，可用 {year} {month} {day} 这类变量",
        152,
        y + 42,
        292,
        0,
    );
    // 这一行占的高度，后面几块的基准都要往下让
    y += 34;

    // 快捷输入说明。码表从 quick 模块取，避免文档和实现对不上。
    // 十一个命令一行放不下（428px 会截断），按四个一行折开。
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

    // 关于：一段自述 + 仓库地址。地址做成按钮而不是纯文字，用户不用手抄 URL。
    // 放在最下面（而不是最上面）是因为它属于「装完看一次就完」的信息，不该挤占
    // 真正要改的设置项。
    let about_y = y + 26 + rows * 20 + 4 + 30;
    // 内容比可视区高是**正常**的（靠滚动看），但总高度得是个合理值：
    // 算错了会得到一条能拖到天涯海角的滚动条，或者底部内容永远滚不到。
    let content_h = about_y + 60;
    debug_assert!(
        (200..4000).contains(&content_h),
        "设置窗内容高度不像话：{content_h}"
    );
    // 存像素值：滚动位置和子窗口坐标都是物理量，CONTENT_H 跟它们同坐标系才比得了。
    CONTENT_H.store(px(content_h), Ordering::Release);
    label(
        h,
        "关于：开源输入法 · Rust + windows-rs 实现 · 不联网、不弹窗、无账号",
        16,
        about_y,
        428,
        0,
    );
    link_label(
        h,
        &format!("GitHub：{ABOUT_URL}"),
        16,
        about_y + 22,
        300,
        ID_ABOUT_LINK,
    );
    push(h, "打开项目主页", 320, about_y + 19, 124, 26, ID_ABOUT);

    // 底部这一栏**不参与滚动**：滚到底时它依然在，任何时候都能关掉窗口。
    //
    // 它的 y 必须从「实际客户区高度」反推，而这个高度是物理值——先换回逻辑值
    // 再减 40，因为 mk() 还会再乘一次 DPI。直接减会得到「150% 下按钮浮在半空」。
    let close_y = dpi::unscale(client_h(h), dpi::dpi()) - 40;
    push(h, "关闭", W - 106, close_y, 90, 30, ID_CLOSE);
    init_scroll(h);
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
            // 只处理点击（普通控件）和**下拉框**的「换了一项」；忽略其它通知噪音。
            //
            // `CBN_SELCHANGE` 必须按 id 收窄到下拉框：它的值是 1，而
            // `STN_DBLCLK` 也是 1。下面那条「GitHub」链接是带 `SS_NOTIFY` 的
            // 静态控件，双击时会依次发来两条 `STN_CLICKED`(0) 和一条
            // `STN_DBLCLK`(1)——不按 id 区分的话三条全都会进 `on_command`，
            // 双击一下打开三个浏览器标签页。
            if code == 0 || (id == ID_ROWS && code == CBN_SELCHANGE) {
                on_command(h, id);
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = ShowWindow(h, SW_HIDE);
            LRESULT(0)
        }
        WM_VSCROLL => {
            let cmd = (w.0 & 0xFFFF) as i32;
            let cur = SCROLL.load(Ordering::Acquire);
            let view = view_h(h);
            let max_pos = (CONTENT_H.load(Ordering::Acquire) - view).max(0);
            // 一行 24 逻辑像素 ≈ 一个复选框行距，滚起来跟得上眼睛
            let line = px(24);
            let target = if cmd == SB_LINEUP.0 {
                cur - line
            } else if cmd == SB_LINEDOWN.0 {
                cur + line
            } else if cmd == SB_PAGEUP.0 {
                cur - view
            } else if cmd == SB_PAGEDOWN.0 {
                cur + view
            } else if cmd == SB_THUMBTRACK.0 || cmd == SB_THUMBPOSITION.0 {
                // 拖滑块时位置在 nTrackPos 里，`WM_VSCROLL` 的 wParam 高字只够 16 位
                // （内容超过 32767px 就会翻符号），所以走 GetScrollInfo 拿全 32 位
                let mut si = SCROLLINFO {
                    cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                    fMask: SIF_TRACKPOS,
                    ..Default::default()
                };
                if GetScrollInfo(h, SB_VERT, &mut si).is_ok() {
                    si.nTrackPos
                } else {
                    cur
                }
            } else if cmd == SB_TOP.0 {
                0
            } else if cmd == SB_BOTTOM.0 {
                max_pos
            } else {
                cur
            };
            scroll_to(h, target);
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            // 高字是有符号的滚动量，一格 120
            let delta = ((w.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
            let step = -(delta / 120) * px(24) * 3;
            scroll_to(h, SCROLL.load(Ordering::Acquire) + step);
            LRESULT(0)
        }
        // 窗口被拖到缩放不同的屏幕上（多屏混缩放时很常见）：整块重排。
        //
        // 布局坐标是逻辑值、`mk()` 里统一乘 DPI，所以重排必须**重建子窗口**——
        // 就地挪动改不了已经下发过的高度和字体（`font::ui()` 也是按 DPI 缓存的，
        // 换屏后要拿新句柄）。重建比「遍历子窗口逐个 SetWindowPos + WM_SETFONT」
        // 少一半代码，而且不会漏掉下拉框这类自绘部分。
        WM_DPICHANGED => {
            dpi::update_for_window(h);
            let mut child = GetWindow(h, GW_CHILD).unwrap_or_default();
            while !child.is_invalid() {
                // 先取下一个再销毁：DestroyWindow 之后 hwnd 就失效了，查不了兄弟
                let next = GetWindow(child, GW_HWNDNEXT).unwrap_or_default();
                let _ = DestroyWindow(child);
                child = next;
            }
            if let Ok(mut g) = SCROLLABLE.lock() {
                g.clear();
            }
            build(h);
            sync(h);
            let (win_w, win_h) = window_size(W, want_client_h());
            let _ = SetWindowPos(
                h,
                None,
                0,
                0,
                win_w,
                win_h,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
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

/// 窗口样式（创建和反算尺寸必须用同一份，否则算出来的高度对不上）。
///
/// `WS_VSCROLL` 常驻：设置项只增不减，内容比屏幕高是常态而不是异常，
/// 与其把窗口撑到屏幕外（1366×768 装不下 900 的客户区），不如让它滚。
const STYLE: WINDOW_STYLE = WINDOW_STYLE(
    WS_OVERLAPPED.0 | WS_CAPTION.0 | WS_SYSMENU.0 | WS_MINIMIZEBOX.0 | WS_VSCROLL.0,
);

/// 由客户区尺寸反算窗口尺寸。标题栏和边框高度随 DPI / 主题变，写死窗口高度
/// 会在高 DPI 下把最下面那排控件截掉。
///
/// 入参是**逻辑**客户区尺寸，返回值是**物理**窗口尺寸：
/// `AdjustWindowRectEx` 和 `GetSystemMetrics` 都在物理坐标系里工作，
/// 混着用会让窗口比内容小一圈（150% 下正好小三分之一）。
unsafe fn window_size(client_w: i32, client_h: i32) -> (i32, i32) {
    let client_w = px(client_w);
    let client_h = px(client_h);
    // **`AdjustWindowRectEx` 不为 `WS_VSCROLL` 留位置**（文档明写：要自己加
    // `SM_CXVSCROLL`）。不补这一条，客户区会窄 17px，右边那列控件被滚动条压掉一截。
    let vscroll = GetSystemMetrics(SM_CXVSCROLL);
    let mut rc = RECT {
        left: 0,
        top: 0,
        right: client_w + vscroll,
        bottom: client_h,
    };
    if AdjustWindowRectEx(&mut rc, STYLE, false, WINDOW_EX_STYLE(0)).is_err() {
        // 反算失败就按 1:1 用，宁可挤一点也不要开不出窗口
        return (client_w + vscroll, client_h);
    }
    (rc.right - rc.left, rc.bottom - rc.top)
}

unsafe fn create() -> Option<HWND> {
    // 先按**光标所在那块屏**定 DPI 再算窗口尺寸：设置窗是从状态浮窗点开的，
    // 光标就在用户想看的那个屏上。多屏混缩放（笔记本 150% + 外接 100%）时，
    // 按主屏算会把窗口做到另一块屏上却按错的尺寸排。
    {
        let mut pt = POINT::default();
        if GetCursorPos(&mut pt).is_err() {
            dpi::update_for_system();
        } else {
            dpi::update_for_point(pt.x, pt.y);
        }
    }
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

    let (win_w, win_h) = window_size(W, want_client_h());
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
            // `SW_RESTORE` 而不是 `SW_SHOW`：后者对已最小化的窗口无效
            // （按「当前大小和位置」显示，最小化的还是最小化），用户看到的是
            // 「点了设置按钮，只有任务栏闪一下」。
            let _ = ShowWindow(existing, SW_RESTORE);
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
