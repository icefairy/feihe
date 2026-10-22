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

    pub fn label(&self) -> &'static str {
        match self {
            Skin::Light => "浅色",
            Skin::Dark => "深色",
            Skin::Eye => "护眼",
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
}

impl Config {
    pub const DEFAULT: Config = Config {
        scheme: Scheme::Xiaohe,
        assist_mode: AssistMode::Ziranma,
        fullwidth: false,
        skin: Skin::Light,
        bar_follow: false,
        bar_x: 0,
        bar_y: 0,
    };
}

static CFG: Mutex<Config> = Mutex::new(Config::DEFAULT);

/// **全拼模式开关——刻意不落盘。**
///
/// 状态栏上那个「全拼」是给访客临时用的，如果写进 config.ini，用户借完电脑忘了切回来，
/// 下次开机就会一脸问号地发现双拼打不出字。所以它只活在本进程里，进程结束即复原。
static PINYIN_MODE: AtomicBool = AtomicBool::new(false);

pub fn pinyin_mode() -> bool {
    PINYIN_MODE.load(Ordering::Acquire)
}

pub fn set_pinyin_mode(on: bool) {
    PINYIN_MODE.store(on, Ordering::Release);
}

pub fn dir() -> PathBuf {
    let base = std::env::var("APPDATA").unwrap_or_else(|_| "C:\\".to_string());
    PathBuf::from(base).join("Feihe")
}

fn path() -> PathBuf {
    dir().join("config.ini")
}

/// 从磁盘读配置到内存。文件不存在就用默认值。
pub fn load() -> Config {
    let mut cfg = Config::DEFAULT;
    if let Ok(text) = std::fs::read_to_string(path()) {
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
                _ => {}
            }
        }
    }
    if let Ok(mut g) = CFG.lock() {
        *g = cfg;
    }
    cfg
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
    let _ = std::fs::create_dir_all(dir());
    let text = format!(
        "; 飞鹤输入法配置\n\
         scheme={}\n\
         assist_mode={}\n\
         fullwidth={}\n\
         skin={}\n\
         bar_follow={}\n\
         bar_x={}\n\
         bar_y={}\n",
        cfg.scheme.key(),
        cfg.assist_mode.key(),
        cfg.fullwidth as u8,
        cfg.skin.key(),
        cfg.bar_follow as u8,
        cfg.bar_x,
        cfg.bar_y
    );
    let _ = std::fs::write(path(), text);
}
