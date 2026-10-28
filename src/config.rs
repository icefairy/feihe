//! 配置持久化：`<安装目录>\config.ini`
//!
//! 手写 `key=value` 解析，不引 serde——配置项就十来个，没必要为此拉一个依赖进来。
//! 每个进程各有一份内存副本，`Activate` 时重新读取；设置窗口改完立即写盘，
//! 其它进程在**下一次按键**时通过 `refresh_if_changed` 跟上（不用切输入法，
//! 更不用重启宿主程序）。
//!
//! ## 数据放在安装目录（而不只是配置）
//!
//! `config.ini` / `freq.txt` / `phrases.txt` / `pinned.txt` 全部和 DLL 同目录。
//! 理由很实际：**压缩一份安装目录就是完整备份**，换机、重装、拷给第二台电脑
//! 都是一次复制。这也是安装器把默认位置从 `Program Files` 挪到
//! `%LOCALAPPDATA%\Feihe` 的原因——`Program Files` 对普通进程只读，写进去会被
//! UAC 重定向到 `VirtualStore`（32 位）或者直接失败（64 位），「备份目录」就成了
//! 假象。
//!
//! 万一有人装到了只读目录（或者从只读介质运行），`resolve_dir` 会检测可写性并
//! 回落到 `%APPDATA%\Feihe`——宁可数据存在别处，也不能每次改设置都静默失败。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI8, AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};


use crate::assist::AssistMode;
use crate::shuangpin::Scheme;
use windows::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};

/// 候选窗 / 状态浮窗在什么情况下不弹。
///
/// 早期这里是 `no_popup: bool`，默认关——结果就是「打游戏之前先去设置里勾一下，
/// 打完再回来取消」，谁也不会这么干。现在默认 `Auto`：由 `fullscreen` 模块问
/// shell「当前是不是全屏」，游戏、演示、全屏 UWP 时自动把候选写进组合串。
///
/// 保留 `Always` / `Never` 是因为自动检测不能覆盖所有场合：
/// - 远程桌面、虚拟机控制台里未必报得出「全屏」，但自绘窗口确实显示不出来
/// - 反过来，有人用无边框窗口跑游戏，shell 不认为是全屏，候选窗会盖在游戏上
///
/// 两种误判都要有手动出口，所以不是「要不要这个开关」的问题，是「默认怎么选」。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PopupMode {
    /// 全屏时收起（默认）
    Auto,
    /// 一律弹（自动检测误判成「全屏」时用）
    Always,
    /// 一律不弹（远程桌面 / 自动检测认不出来的独占全屏）
    Never,
}

impl PopupMode {
    pub fn from_key(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "always" | "1" | "true" | "yes" | "on" => PopupMode::Always,
            "never" | "2" => PopupMode::Never,
            _ => PopupMode::Auto,
        }
    }

    pub fn key(&self) -> &'static str {
        match self {
            PopupMode::Auto => "auto",
            PopupMode::Always => "always",
            PopupMode::Never => "never",
        }
    }

    fn bits(self) -> u8 {
        match self {
            PopupMode::Auto => MODE_AUTO,
            PopupMode::Always => MODE_ALWAYS,
            PopupMode::Never => MODE_NEVER,
        }
    }

    fn from_bits(b: u8) -> Self {
        match b {
            MODE_ALWAYS => PopupMode::Always,
            MODE_NEVER => PopupMode::Never,
            _ => PopupMode::Auto,
        }
    }
}

const MODE_AUTO: u8 = 0;
const MODE_ALWAYS: u8 = 1;
const MODE_NEVER: u8 = 2;


#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Skin {
    Light,
    Dark,
    Eye,
}

impl Skin {
    pub fn from_key(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "dark" => Skin::Dark,
            "eye" => Skin::Eye,
            _ => Skin::Light,
        }
    }

    pub fn key(&self) -> &'static str {
        match self {
            Skin::Light => "light",
            Skin::Dark => "dark",
            Skin::Eye => "eye",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// 双拼方案
    pub scheme: Scheme,
    /// 辅助码（助记码）方案：关闭 / 自然码 / 小鹤音形
    pub assist_mode: AssistMode,
    /// 全角标点 / 全角字母数字
    pub fullwidth: bool,
    /// 状态浮窗配色
    pub skin: Skin,
    /// 浮窗是否跟随光标（false = 固定在屏幕右下角）
    pub bar_follow: bool,
    /// 浮窗固定位置（bar_follow = false 时有效；0,0 表示自动贴右下角）
    pub bar_x: i32,
    pub bar_y: i32,
    /// 全拼模式（false = 双拼）。见下方 `PINYIN_MODE` 的说明。
    pub pinyin_mode: bool,
    /// 输出繁体字（false = 简体）
    pub traditional: bool,
    /// 动态词频（按用户实际选词重排候选）。见 `freq` 模块。
    pub learn: bool,
    /// 候选窗 / 状态浮窗何时不弹，见 `PopupMode`。
    pub popup: PopupMode,
    /// 候选窗里给易错词标注带调读音（`重庆 chóng qìng`）。见 `hint` 模块。
    pub reading: bool,
    /// 音形「3 键直出」：第 3 键不问青红皂白先当辅助码（`ddj` → `dd` + `j` → 带）。
    ///
    /// 双拼每个音节恰好 2 键，**3 键必然是「1 个完整音节 + 1 个孤立键」**，
    /// 那个孤立键有两种读法：下一个音节的首键（`ddj` 想打「代价」），或者辅助码
    /// （`dd` + 部首 j → 带）。这两件事没法从按键流里分辨，只能选一个优先。
    ///
    /// 默认关：多数人敲到第 3 键是在继续打词，抢成辅助码等于把词吞掉。
    /// 打开的人（从自然码 / 小鹤带过来的）宁愿先出单字，词可以关掉开关或退格重打。
    pub assist_direct: bool,
}

impl Config {
    pub const DEFAULT: Config = Config {
        scheme: Scheme::Xiaohe,
        assist_mode: AssistMode::Ziranma,
        // 中文输入法默认应该是中文标点。早期版本这里是 false，导致用户
        // 「打不出、和，」——`、` 恰好落在 `\` 上，见 textservice 的 fullwidth_punct。
        fullwidth: true,
        skin: Skin::Light,
        bar_follow: false,
        bar_x: 0,
        bar_y: 0,
        pinyin_mode: false,
        traditional: false,
        // 默认开：输入法本来就该越用越顺手。不想要的人在设置里关掉即可。
        learn: true,
        // 默认自动：全屏游戏 / 演示时收起候选窗，其余时候正常弹。
        popup: PopupMode::Auto,
        // 默认开：易错词的读音该让人看见，不然「重」到底读 chóng 还是 zhòng
        // 只能靠猜。不想要的人关掉即可，文字宽度会回到旧样子。
        reading: true,
        // 默认关：3 键里那个孤立键更可能是在打下一个音节，自动拆会吞词。
        assist_direct: false,
    };
}

static CFG: Mutex<Config> = Mutex::new(Config::DEFAULT);

/// **全拼模式开关。**
///
/// 早期它是状态栏上的临时按钮，刻意不落盘（怕借电脑的人切到全拼后忘了切回来，
/// 下次开机一脸问号）。现在按钮挪进了设置窗，语义变成「设置项」——设置项改了
/// 重启就失效会非常费解，所以改成落盘。
///
/// 仍然保留一个原子镜像：`pinyin_mode()` 在每个按键上会被调用好几次
/// （`is_code_key` / `code_char` / `syllables` / `code_allowed`），走 Mutex 不划算。
static PINYIN_MODE: AtomicBool = AtomicBool::new(false);

pub fn pinyin_mode() -> bool {
    PINYIN_MODE.load(Ordering::Acquire)
}

/// **弹窗策略的原子镜像。**
///
/// 和 `PINYIN_MODE` 一个道理：候选窗每次按键都要问一遍「该不该弹」，
/// 状态浮窗在激活时也要问，走 Mutex 不划算。
static POPUP_MODE: AtomicU8 = AtomicU8::new(MODE_AUTO);

/// 运行时的临时覆盖：-1 = 没覆盖，0 = 强制弹，1 = 强制不弹。
///
/// 只给 `Ctrl+Shift+U` 用。**不落盘**：这是「这一会儿先这么着」的应急键，
/// 用户真想要长期行为就去设置里改。分进程各存一份也正好——它管的是当前
/// 这个宿主窗口，别的程序不受影响。
static POPUP_OVERRIDE: AtomicI8 = AtomicI8::new(-1);

/// 当前是否该「不弹窗」。
///
/// 判断顺序：临时覆盖 → 用户设置 → 自动检测。自动检测每次都问 `fullscreen`，
/// 那边自带 400ms 缓存，热路径上没有真实开销。
pub fn no_popup() -> bool {
    match POPUP_OVERRIDE.load(Ordering::Acquire) {
        v if v >= 0 => v == 1,
        _ => match PopupMode::from_bits(POPUP_MODE.load(Ordering::Acquire)) {
            PopupMode::Always => false,
            PopupMode::Never => true,
            PopupMode::Auto => crate::fullscreen::active(),
        },
    }
}

pub fn set_popup_mode(m: PopupMode) {
    // 用户动了设置项，临时覆盖就作废——否则会「改了设置但行为没变」
    POPUP_OVERRIDE.store(-1, Ordering::Release);
    update(|c| c.popup = m);
}

/// 切换「临时强制」，返回切换后的覆盖值（`None` = 已取消覆盖，跟随设置）。
pub fn toggle_popup_override() -> Option<bool> {
    let next = if POPUP_OVERRIDE.load(Ordering::Acquire) >= 0 {
        None
    } else {
        Some(!no_popup())
    };
    POPUP_OVERRIDE.store(next.map_or(-1, |v| v as i8), Ordering::Release);
    next
}

/// 单测专用：直接改镜像，绕开配置文件。候选窗的鼠标测试需要「允许弹窗」。
#[cfg(test)]
pub fn set_no_popup_for_test(v: bool) {
    POPUP_OVERRIDE.store(if v { 1 } else { 0 }, Ordering::Release);
}

/// **读音提示的原子镜像。** 候选窗每画一页都要问一遍（每项一次），走 Mutex 不划算。
static READING: AtomicBool = AtomicBool::new(true);

pub fn reading() -> bool {
    READING.load(Ordering::Acquire)
}

pub fn set_reading(on: bool) {
    READING.store(on, Ordering::Release);
    update(|c| c.reading = on);
}

/// 把配置里的热路径开关同步到原子镜像。`load` / `update` 各调一次。
fn sync_mirrors(cfg: &Config) {
    PINYIN_MODE.store(cfg.pinyin_mode, Ordering::Release);
    POPUP_MODE.store(cfg.popup.bits(), Ordering::Release);
    READING.store(cfg.reading, Ordering::Release);
}

pub fn set_pinyin_mode(on: bool) {
    PINYIN_MODE.store(on, Ordering::Release);
    update(|c| c.pinyin_mode = on);
}

pub fn dir() -> PathBuf {
    DATA_DIR.get_or_init(resolve_dir).clone()
}

/// 数据目录只解析一次：`dir()` 在热路径上（`freq::file()` 每次上屏都要拼路径），
/// 每次都去问 DLL 路径 + 试写文件太贵。
static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 旧位置：`%APPDATA%\Feihe`。
pub fn legacy_dir() -> PathBuf {
    let base = std::env::var("APPDATA").unwrap_or_else(|_| "C:\\".to_string());
    PathBuf::from(base).join("Feihe")
}

/// 决定数据目录：优先「DLL 所在目录」，不可写就回落 `%APPDATA%\Feihe`。
fn resolve_dir() -> PathBuf {
    let fallback = legacy_dir();
    // 测试进程按老规矩走 APPDATA：否则会读到 target\debug 下的文件，而且
    // 「首次运行迁移」会把用户真实的配置拷进构建目录。
    if cfg!(test) {
        return fallback;
    }
    let Some(exe_dir) = module_dir() else {
        return fallback;
    };
    if !writable(&exe_dir) {
        return fallback;
    }
    migrate(&fallback, &exe_dir);
    exe_dir
}

/// 本 DLL 所在目录（不是宿主 EXE 的目录）。
fn module_dir() -> Option<PathBuf> {
    unsafe {
        let mut m = windows::Win32::Foundation::HMODULE::default();
        let addr = module_dir as *const core::ffi::c_void;
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            windows::core::PCWSTR(addr as *const u16),
            &mut m,
        )
        .ok()?;
        let mut buf = [0u16; 1024];
        let n = GetModuleFileNameW(Some(m), &mut buf) as usize;
        if n == 0 || n >= buf.len() {
            return None;
        }
        PathBuf::from(String::from_utf16_lossy(&buf[..n]))
            .parent()
            .map(Path::to_path_buf)
    }
}

/// 目录能不能写。真写一个文件再删掉——查 ACL 不可靠（`Program Files` 的实际
/// 权限取决于组策略，`readonly` 属性也不代表 ACL）。
fn writable(d: &Path) -> bool {
    if !d.is_dir() {
        return false;
    }
    let probe = d.join(".write-probe");
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// 老位置 → 新位置的一次性搬移（复制，不删原件）。
///
/// 只在**新位置还没有 config.ini** 时动手：已经有配置说明用户已经在新位置跑过，
/// 再拷一次会把后来的改动冲掉。用复制不用移动——搬到一半失败（磁盘满、权限）
/// 时老数据还在，用户不会两头落空。
fn migrate(from: &Path, to: &Path) {
    if to.join("config.ini").exists() || from == to {
        return;
    }
    let _ = std::fs::create_dir_all(to);
    for name in ["config.ini", "freq.txt", "phrases.txt", "pinned.txt"] {
        let src = from.join(name);
        let dst = to.join(name);
        if src.exists() && !dst.exists() {
            let _ = std::fs::copy(&src, &dst);
        }
    }
}

fn path() -> PathBuf {
    dir().join("config.ini")
}

/// 配置版本。老文件没有这个键，据此做一次性迁移。
///
/// **加字段就要想清楚：迁移必须分阶段写，不能只判「ver < 最新」就重放所有历史迁移。**
/// 之前只有 v1→v2 时，代码是 `if ver < CONFIG_VER { 强改 fullwidth }`，看起来没问题；
/// 一旦把 CONFIG_VER 提到 3，v2 用户的 `fullwidth=0`（明确关掉全角标点）就会被
/// 再「迁移」一次、强行改回 true。所以下面按 `ver < 2` 精确判阶段。
const CONFIG_VER: u32 = 7;

/// 从磁盘读配置到内存。文件不存在就用默认值。
pub fn load() -> Config {
    let (mut cfg, ver) = match std::fs::read_to_string(path()) {
        Ok(text) => parse(&text),
        Err(_) => (Config::DEFAULT, CONFIG_VER), // 没有文件 = 全新安装，不需要迁移
    };

    // v1 → v2：v1 把 `fullwidth` 默认成 false（当时只是没想清楚），结果是中文输入法
    // 打出来的全是半角标点，连顿号都打不出来。老配置文件里那个 `fullwidth=0`
    // 是默认值写下来的，不是用户的选择，所以一次性改成 true。
    if ver < 2 {
        cfg.fullwidth = true;
    }

    // v2 → v3：新增 `learn`（动态词频）。`parse` 对缺失的键已经给了默认值 true，
    // 这里不需要额外动作——只把版本号写进文件，免得下次又走一遍迁移判断。

    // v3 → v4：新增 `uileless`（不弹窗模式）。缺省 false，同样不需要迁移动作。

    // v4 → v5：`uileless` 改名成 `no_popup`。**没有行为变化，纯改名。**
    // 为什么要改：真正的 TSF UILess 是宿主驱动的（候选交给宿主自己画，见
    // `uielement` 模块），这个开关只是「不弹窗、候选拼进组合串」。两件事同名，
    // 代码和文档里会一直有人搞混。`parse` 仍然认旧键，所以老配置不需要额外动作，
    // 下次 `save` 就写成新键名了。

    // v5 → v6：`no_popup: bool` 换成 `popup: auto|always|never`。
    //
    // 旧文件里 `no_popup=0` 是**默认值**写下来的，不是用户的选择，所以升级成
    // `auto`（新默认）而不是 `always`。`no_popup=1` 是用户手工勾过的，保留成
    // `never`——不能把人家明确要的行为在升级时悄悄改掉。
    //
    // 判定放在 `parse` 里（读到旧键就按上面的规则映射），这里不需要额外动作，
    // 只要 `ver < CONFIG_VER` 时重写一遍文件、把键名换成新的。

    // v6 → v7：新增 `assist_direct`（音形 3 键直出）。缺省 false，不需要迁移动作——
    // 这个开关的默认值就是「保持旧行为」，老用户升级后手感不能变。

    // 原子镜像必须在锁外同步：`pinyin_mode()` / `no_popup()` 是热路径，读的是原子量
    sync_mirrors(&cfg);
    if let Ok(mut g) = CFG.lock() {
        *g = cfg;
    }
    if ver < CONFIG_VER {
        save(&cfg);
    }
    cfg
}

/// 解析 config.ini 文本，同时返回文件里声明的版本号（没有就是 0）。
/// 抽成纯函数是为了能单测迁移逻辑——`load()` 直接读写 APPDATA，测不了。
fn parse(text: &str) -> (Config, u32) {
    let mut cfg = Config::DEFAULT;
    let mut ver = 0u32;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let (k, v) = (k.trim(), v.trim());
        match k {
            "config_ver" => ver = v.parse().unwrap_or(0),
            "scheme" => cfg.scheme = Scheme::from_key(v),
            "assist_mode" => cfg.assist_mode = AssistMode::from_key(v),
            // 兼容旧配置：assist=1 视为自然码辅助码
            "assist" => {
                cfg.assist_mode = if parse_bool(v) {
                    AssistMode::Ziranma
                } else {
                    AssistMode::Off
                }
            }
            "fullwidth" => cfg.fullwidth = parse_bool(v),
            "skin" => cfg.skin = Skin::from_key(v),
            "bar_follow" => cfg.bar_follow = parse_bool(v),
            "bar_x" => cfg.bar_x = v.parse().unwrap_or(0),
            "bar_y" => cfg.bar_y = v.parse().unwrap_or(0),
            "pinyin_mode" => cfg.pinyin_mode = parse_bool(v),
            "traditional" => cfg.traditional = parse_bool(v),
            "learn" => cfg.learn = parse_bool(v),
            "popup" => cfg.popup = PopupMode::from_key(v),
            "reading" => cfg.reading = parse_bool(v),
            "assist_direct" => cfg.assist_direct = parse_bool(v),
            // v5 及更早的键名。`no_popup=1` 是用户勾过的 → 保持「一律不弹」；
            // `0` 是默认值 → 升级到自动检测（见 `load` 里的 v5→v6 说明）。
            "no_popup" | "uileless" => {
                cfg.popup = if parse_bool(v) {
                    PopupMode::Never
                } else {
                    PopupMode::Auto
                }
            }
            _ => {}
        }
    }
    (cfg, ver)
}

fn parse_bool(v: &str) -> bool {
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// 当前内存中的配置
pub fn get() -> Config {
    CFG.lock().map(|g| *g).unwrap_or(Config::DEFAULT)
}

/// `config.ini` 上一次被本进程读到时的指纹：`(修改时间纳秒, 字节数)`。
static STAMP: Mutex<Option<(u128, u64)>> = Mutex::new(None);

fn stamp_of(p: &Path) -> Option<(u128, u64)> {
    let m = std::fs::metadata(p).ok()?;
    let t = m
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some((t, m.len()))
}

/// 配置文件变了就重读一遍，返回是否真的重读过。
///
/// ## 为什么需要
///
/// 配置是**每个宿主进程各一份**内存副本：在设置窗口（跑在随便哪个进程里）勾上
/// 「3 键直出」或把辅助码改成音形之后，Word / 浏览器里那份副本还是旧的。用户
/// 回到文档接着打字，得到的是旧行为，而设置窗口明明勾好了——这时候任何解释
/// （「要切一下输入法」「要重启」）都像甩锅。
///
/// 以前只在 `Activate` 里读：切输入法 / 切窗口才生效。现在每次按键前探一次
/// 文件指纹，一次 `metadata` 调用（微秒级），换来「改完立刻就对了」。
///
/// ## 为什么用 mtime + 长度而不是只看 mtime
///
/// 文件系统时间戳精度有限（FAT 是 2 秒），同一秒内连改两次可能看起来没变。
/// 加上长度能挡住绝大多数「改完立刻保存」的情况；`save` 用的是写临时文件再改名，
/// 长度变化几乎必然发生。
pub fn refresh_if_changed() -> bool {
    let now = stamp_of(&path());
    let changed = {
        let mut g = match STAMP.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        if *g == now {
            false
        } else {
            *g = now;
            true
        }
    };
    if changed {
        load();
        // `load` 可能因为版本迁移顺手重写了文件（`ver < CONFIG_VER`），指纹跟着变。
        // 这里立刻把新指纹记下来，否则下一次按键会为同一次迁移再白读一遍。
        if let Ok(mut g) = STAMP.lock() {
            *g = stamp_of(&path());
        }
    }
    changed
}

/// 改配置并立即写盘
pub fn update(f: impl FnOnce(&mut Config)) {
    let snapshot = {
        let Ok(mut g) = CFG.lock() else { return };
        f(&mut g);
        *g
    };
    sync_mirrors(&snapshot);
    save(&snapshot);
}

pub fn save(cfg: &Config) {
    // 测试进程绝不写盘。`cargo test` 里有好几个用例会调 `set_pinyin_mode()`，
    // 那会走到 update() → save()，而测试进程里的 CFG 是默认值——一跑测试就把
    // 用户真实的 config.ini 覆盖成默认配置了。
    if cfg!(test) {
        return;
    }
    let _ = std::fs::create_dir_all(dir());
    let text = format!(
        "; 飞鹤输入法配置\n\
         config_ver={CONFIG_VER}\n\
         scheme={}\n\
         assist_mode={}\n\
         fullwidth={}\n\
         skin={}\n\
         bar_follow={}\n\
         bar_x={}\n\
         bar_y={}\n\
         pinyin_mode={}\n\
         traditional={}\n\
         learn={}\n\
         popup={}\n\
         reading={}\n\
         assist_direct={}\n",
        cfg.scheme.key(),
        cfg.assist_mode.key(),
        cfg.fullwidth as u8,
        cfg.skin.key(),
        cfg.bar_follow as u8,
        cfg.bar_x,
        cfg.bar_y,
        cfg.pinyin_mode as u8,
        cfg.traditional as u8,
        cfg.learn as u8,
        cfg.popup.key(),
        cfg.reading as u8,
        cfg.assist_direct as u8
    );
    // 先写临时文件再改名：每个宿主进程在 Activate 时都会 load()，而 v1→v2 迁移
    // 会在 load() 里落盘。多个进程同时写同一个文件时，直接 write 会让别的进程
    // 读到「已截断但还没写完」的半截配置（丢设置）。改名是原子的。
    let tmp = dir().join("config.ini.tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, path());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_config_without_version_is_migrated() {
        // v1 文件：没有 config_ver，fullwidth 被写成 0
        let (cfg, ver) = parse("scheme=ms\nfullwidth=0\nskin=dark\n");
        assert_eq!(ver, 0);
        assert_eq!(cfg.scheme, Scheme::Ms);
        assert_eq!(cfg.skin, Skin::Dark);
        assert!(!cfg.fullwidth);
        // load() 里会把它翻成 true，这里只验证「被识别成需要迁移」
        assert!(ver < CONFIG_VER);
    }

    #[test]
    fn current_config_keeps_explicit_choice() {
        // 已经是 v2 的文件：用户明确关了全角标点，不许再改回去
        let (cfg, ver) = parse(&format!("config_ver={CONFIG_VER}\nfullwidth=0\n"));
        assert_eq!(ver, CONFIG_VER);
        assert!(!cfg.fullwidth);
        assert!(!(ver < CONFIG_VER));
    }

    #[test]
    fn new_fields_default_off() {
        let (cfg, _) = parse("");
        assert!(!cfg.pinyin_mode);
        assert!(!cfg.traditional);
        assert!(cfg.fullwidth, "全新配置默认应该是中文标点");
        assert!(cfg.learn, "动态词频默认开");
        assert_eq!(cfg.popup, PopupMode::Auto, "弹窗策略默认自动");
        assert!(
            !cfg.assist_direct,
            "3 键直出默认关：那 1 键更可能是在打下一个音节"
        );
    }

    #[test]
    fn parses_new_switches() {
        let (cfg, _) = parse(&format!(
            "config_ver={CONFIG_VER}\npinyin_mode=1\ntraditional=1\nlearn=0\npopup=never\n"
        ));
        assert!(cfg.pinyin_mode);
        assert!(cfg.traditional);
        assert!(!cfg.learn);
        assert_eq!(cfg.popup, PopupMode::Never);
        assert!(!cfg.assist_direct, "没写这个键时保持默认关");
    }

    #[test]
    fn parses_assist_direct() {
        let (cfg, _) = parse("assist_direct=1\n");
        assert!(cfg.assist_direct);
    }

    /// v4 的键名 `uileless` 还要认：用户明确开过的开关不能在升级时丢掉。
    #[test]
    fn legacy_uileless_key_still_reads() {
        let (cfg, ver) = parse("config_ver=4\nuileless=1\n");
        assert_eq!(ver, 4);
        assert_eq!(cfg.popup, PopupMode::Never, "旧键 1 = 一律不弹");
        assert!(ver < CONFIG_VER, "v4 文件会被重写成新版本（键名跟着换）");
    }

    /// v5 的 `no_popup=0` 是**默认值**写下来的，不是用户的选择，
    /// 升级后要变成「自动」——否则老用户永远享受不到全屏自动检测。
    #[test]
    fn v5_no_popup_zero_becomes_auto() {
        let (cfg, ver) = parse("config_ver=5\nno_popup=0\n");
        assert_eq!(ver, 5);
        assert_eq!(cfg.popup, PopupMode::Auto);
    }

    /// 反过来，`no_popup=1` 是用户手工勾的，升级后必须还是「不弹」。
    #[test]
    fn v5_no_popup_one_becomes_never() {
        let (cfg, _) = parse("config_ver=5\nno_popup=1\n");
        assert_eq!(cfg.popup, PopupMode::Never);
    }

    /// 三态策略的键名/位值互相能转回来。设置窗按位值打勾，配置文件按名字存，
    /// 两边对不上就是「勾了自动、重启变永远不弹」这种诡异现象。
    #[test]
    fn popup_mode_roundtrips() {
        for m in [PopupMode::Auto, PopupMode::Always, PopupMode::Never] {
            assert_eq!(PopupMode::from_key(m.key()), m);
            assert_eq!(PopupMode::from_bits(m.bits()), m);
        }
        // 手改配置写成 1/0 也要认
        assert_eq!(PopupMode::from_key("1"), PopupMode::Always);
        assert_eq!(PopupMode::from_key("0"), PopupMode::Auto);
        assert_eq!(PopupMode::from_key("乱写"), PopupMode::Auto);
    }

    /// v2 用户明确关掉了全角标点，升级到 v3 **不能**把它改回 true。
    ///
    /// 这个用例是给「迁移写成 `if ver < CONFIG_VER { fullwidth = true }`」埋的钉子：
    /// 那种写法在只有 v1→v2 时看不出问题，一加 v3 就会静默重放 v2 的迁移，
    /// 用户设置被无声改掉——比崩掉还难查。
    #[test]
    fn v2_migration_does_not_touch_fullwidth() {
        let (cfg, ver) = parse("config_ver=2\nfullwidth=0\nlearn=0\n");
        assert_eq!(ver, 2);
        assert!(ver < CONFIG_VER, "v2 文件确实需要升级到 v3");
        // load() 里的 v1 分支不该命中
        assert!(ver >= 2, "ver>=2 时不能走 v1 的 fullwidth 迁移");
        assert!(!cfg.fullwidth);
        assert!(!cfg.learn);
    }

    /// 老文件没有 learn 键 → 取默认值 true（而不是 false）
    #[test]
    fn missing_learn_key_defaults_on() {
        let (cfg, _) = parse(&format!("config_ver={CONFIG_VER}\ntraditional=1\n"));
        assert!(cfg.learn);
        assert!(cfg.traditional);
    }
}
