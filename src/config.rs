//! 配置持久化：`%APPDATA%\Feihe\config.ini`
//!
//! 手写 `key=value` 解析，不引 serde——配置项就十来个，没必要为此拉一个依赖进来。
//! 每个进程各有一份内存副本，`Activate` 时重新读取；设置窗口改完立即写盘，
//! 其它进程下次激活时自然生效。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::assist::AssistMode;
use crate::shuangpin::Scheme;

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

pub fn set_pinyin_mode(on: bool) {
    PINYIN_MODE.store(on, Ordering::Release);
    update(|c| c.pinyin_mode = on);
}

pub fn dir() -> PathBuf {
    let base = std::env::var("APPDATA").unwrap_or_else(|_| "C:\\".to_string());
    PathBuf::from(base).join("Feihe")
}

fn path() -> PathBuf {
    dir().join("config.ini")
}

/// 配置版本。老文件没有这个键，据此做一次性迁移。
const CONFIG_VER: u32 = 2;

/// 从磁盘读配置到内存。文件不存在就用默认值。
pub fn load() -> Config {
    let (cfg, ver) = match std::fs::read_to_string(path()) {
        Ok(text) => parse(&text),
        Err(_) => (Config::DEFAULT, CONFIG_VER), // 没有文件 = 全新安装，不需要迁移
    };

    // v1 → v2：v1 把 `fullwidth` 默认成 false（当时只是没想清楚），结果是中文输入法
    // 打出来的全是半角标点，连顿号都打不出来。老配置文件里那个 `fullwidth=0`
    // 是默认值写下来的，不是用户的选择，所以一次性改成 true。
    // 迁移后文件里就有 config_ver=2 了，用户之后再手动关掉不会被反复改回去。
    let migrate = ver < CONFIG_VER;
    let cfg = if migrate {
        Config {
            fullwidth: true,
            ..cfg
        }
    } else {
        cfg
    };

    // 原子镜像必须在锁外同步：`pinyin_mode()` 是热路径，读的是这个原子量
    PINYIN_MODE.store(cfg.pinyin_mode, Ordering::Release);
    if let Ok(mut g) = CFG.lock() {
        *g = cfg;
    }
    if migrate {
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

/// 改配置并立即写盘
pub fn update(f: impl FnOnce(&mut Config)) {
    let snapshot = {
        let Ok(mut g) = CFG.lock() else { return };
        f(&mut g);
        *g
    };
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
         traditional={}\n",
        cfg.scheme.key(),
        cfg.assist_mode.key(),
        cfg.fullwidth as u8,
        cfg.skin.key(),
        cfg.bar_follow as u8,
        cfg.bar_x,
        cfg.bar_y,
        cfg.pinyin_mode as u8,
        cfg.traditional as u8
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
    }

    #[test]
    fn parses_new_switches() {
        let (cfg, _) = parse(&format!(
            "config_ver={CONFIG_VER}\npinyin_mode=1\ntraditional=1\n"
        ));
        assert!(cfg.pinyin_mode);
        assert!(cfg.traditional);
    }
}
