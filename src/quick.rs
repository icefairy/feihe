//! 快捷输入：日期 / 时间 / 星期 / 时间戳 / 农历 / 干支 / 节气
//!
//! ## 为什么要有引导符
//!
//! 搜狗是**全拼**，所以 `rq`、`sj` 这种两字母码不会撞车。飞鹤是**双拼**，两字母
//! 恰好就是一个音节：`sj` = `s`+`an` = `san`（三），`xq` = `x`+`iu` = `xiu`（休）。
//! 直接照搬会有一半命令被词库吃掉，行为不可预测。
//!
//! 所以统一用引导符引导：`/rq` 日期、`/sj` 时间。引导符不属于任何双拼方案，
//! 也不会出现在全拼音节里，永远不会有歧义。
//!
//! ## 引导符为什么是 `/`
//!
//! 最初用 `\`，但 `\` 是搜狗用户的「顿号键」——中文模式下按 `\` 就该出 `、`。
//! 引导符占着它，`、` 就打不出来了。
//!
//! 中间一度搬到反引号（`` ` ``），理由是「`/` 和 `\` 都出顿号」。后来改回 `/`：
//! 顿号留一个键就够（`\` 是搜狗习惯，也是绝大多数人的第一反应），而 `/` 位置好记、
//! 好按，比反引号适合当功能入口。
//!
//! 代价是中文模式下 `/` 不再出 `、`（也不会出 `/`），想打半角斜杠就敲 `/` 再按回车
//! （见 `textservice::commit_raw`）。`·`（原来在反引号上）仍在符号软键盘里。
//!
//! ## 前缀匹配
//!
//! 八个命令的首字母互不相同（r / s / x / d / t / n / g / j），所以 `/r` 就能唯一
//! 确定「日期」。打 `/rq` 和 `/r` 效果一样，不要求用户记全。
//!
//! 只打一个引导符时列出全部命令的**当前值预览**——按空格得到最常用的日期，按数字选别的。
//! 这样不用记码也能用。
//!
//! ## 农历 / 干支 / 节气来自 Tyme
//!
//! 这三样都要「历法级」正确：农历要处理闰月、干支要按节气换月、四柱要按子时换日。
//! 自己写等于重造一遍万年历，且几乎必然在边界上错。所以统一用
//! [Tyme](https://6tail.cn/tyme.html)（`tyme4rs`，MIT）：它的节气算法引自寿星天文历，
//! 农历和干支用的是同一套儒略日，三者不会互相打架。
//!
//! 时间基准是**本机本地时间**（`GetLocalTime`），不做时区换算——用户问「今天农历几号」
//! 问的是他眼前这台电脑的今天。

use std::sync::Mutex;

use tyme4rs::tyme::solar::{SolarDay, SolarTime};
use tyme4rs::tyme::{Culture, Tyme};
use windows::Win32::System::SystemInformation::GetLocalTime;

/// 引导符。见模块文档：双拼两字母会和音节撞车，必须有引导符；
/// 顿号留给 `\` 一个键，`/` 位置好按，拿来当功能入口。
pub const PREFIX: char = '/';

/// 一次取到的时间快照。同一批候选必须用同一个时刻，否则「17:36」和「17:36:50」
/// 可能跨秒对不上。
struct Now {
    year: u16,
    month: u16,
    day: u16,
    /// 0 = 周日
    weekday: u16,
    hour: u16,
    minute: u16,
    second: u16,
    /// Unix 秒
    epoch: u64,
    /// Unix 毫秒
    epoch_ms: u128,
}

impl Now {
    fn local() -> Self {
        let st = unsafe { GetLocalTime() };
        let (epoch, epoch_ms) = match std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
        {
            Ok(d) => (d.as_secs(), d.as_millis()),
            Err(_) => (0, 0),
        };
        Self {
            year: st.wYear,
            month: st.wMonth,
            day: st.wDay,
            weekday: st.wDayOfWeek,
            hour: st.wHour,
            minute: st.wMinute,
            second: st.wSecond,
            epoch,
            epoch_ms,
        }
    }

    fn date_cn(&self) -> String {
        format!("{}年{}月{}日", self.year, self.month, self.day)
    }

    fn date_dash(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    fn time_hms(&self) -> String {
        format!("{:02}:{:02}:{:02}", self.hour, self.minute, self.second)
    }

    fn time_hm(&self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }

    fn weekday_cn(&self) -> &'static str {
        const NAMES: [&str; 7] = [
            "星期日", "星期一", "星期二", "星期三", "星期四", "星期五", "星期六",
        ];
        NAMES[(self.weekday as usize).min(6)]
    }

    fn weekday_short(&self) -> &'static str {
        const NAMES: [&str; 7] = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];
        NAMES[(self.weekday as usize).min(6)]
    }

    /// 12 小时制的钟点，`17` → `5`
    fn hour12(&self) -> u16 {
        let h = self.hour % 12;
        if h == 0 {
            12
        } else {
            h
        }
    }

    fn ampm(&self) -> &'static str {
        if self.hour < 12 {
            "上午"
        } else {
            "下午"
        }
    }
}

/// 农历 / 干支 / 节气的缓存。
///
/// 这三样走 Tyme 的天文算法，比读一次系统时间贵；而 `candidates()` 每敲一个键
/// 都会重算（只打引导符时更是要把 8 个命令全列出来）。同一分钟内的输出完全一样
/// ——农历只到日、四柱的时柱两小时才换——所以按「年月日时分」缓存，三样一起算。
static CACHE: Mutex<Option<CacheEntry>> = Mutex::new(None);

struct CacheEntry {
    stamp: String,
    lunar: Vec<String>,
    ganzhi: Vec<String>,
    term: Vec<String>,
}

/// 取缓存；分钟变了就三样一起重算。`pick` 决定要哪一份。
fn cached(n: &Now, pick: fn(&CacheEntry) -> &Vec<String>) -> Vec<String> {
    let stamp = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        n.year, n.month, n.day, n.hour, n.minute
    );
    let mut g = match CACHE.lock() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    if !g.as_ref().is_some_and(|e| e.stamp == stamp) {
        *g = Some(CacheEntry {
            lunar: lunar_candidates(n),
            ganzhi: ganzhi_candidates(n),
            term: term_candidates(n),
            stamp,
        });
    }
    pick(g.as_ref().expect("刚填过")).clone()
}

/// Tyme 的 `SolarDay`：本地时间的年月日直接当公历，不做时区换算。
fn solar_day(n: &Now) -> SolarDay {
    SolarDay::from_ymd(n.year as isize, n.month as usize, n.day as usize)
}

/// 农历：农历丙午年八月廿三 / 丙午年八月廿三 / 八月廿三
///
/// 干支年用**农历年**的干支（按春节换年），不是四柱的年柱（按立春换年）——
/// 「今年什么年」问的是农历。
fn lunar_candidates(n: &Now) -> Vec<String> {
    let day = solar_day(n).get_lunar_day();
    // 干支年用**农历年**的干支（按春节换年），不是四柱的年柱（按立春换年）——
    // 「今年什么年」问的是农历。`get_year_sixty_cycle` 已废弃，走 SixtyCycleDay。
    let gz = day.get_sixty_cycle_day().get_year().to_string();
    let month = day.get_lunar_month().get_name(); // 八月 / 闰六月
    let d = day.get_name(); // 廿三
    vec![
        format!("农历{gz}年{month}{d}"),
        format!("{gz}年{month}{d}"),
        format!("{month}{d}"),
    ]
}

/// 干支四柱：丙午年 乙酉月 壬寅日 丁未时
///
/// 交给 Tyme 的 `EightChar`：月柱按节气分界、日柱按子时（23:00）换日，
/// 这两条自己写几乎必错。
fn ganzhi_candidates(n: &Now) -> Vec<String> {
    let time = SolarTime::from_ymd_hms(
        n.year as isize,
        n.month as usize,
        n.day as usize,
        n.hour as usize,
        n.minute as usize,
        n.second as usize,
    );
    let ec = time.get_lunar_hour().get_eight_char();
    let (y, m) = (ec.get_year().to_string(), ec.get_month().to_string());
    let (d, h) = (ec.get_day().to_string(), ec.get_hour().to_string());
    vec![
        format!("{y}年 {m}月 {d}日 {h}时"),
        format!("{y} {m} {d} {h}"),
        format!("{y}年 {m}月 {d}日"),
    ]
}

/// 下一个节气（精确到日）：2026年11月7日 立冬
///
/// `get_term_day()` 给的是「今天所在的节气 + 已经过了几天」，所以 `day_index == 0`
/// 说明今天就是节气日——那就是最近的一个，不用往后跳。
fn next_term_day(n: &Now) -> (SolarDay, String) {
    let td = solar_day(n).get_term_day();
    let term = if td.get_day_index() == 0 {
        td.get_solar_term()
    } else {
        td.get_solar_term().next(1)
    };
    (term.get_solar_day(), term.get_name())
}

fn term_candidates(n: &Now) -> Vec<String> {
    let (day, name) = next_term_day(n);
    vec![
        format!(
            "{}年{}月{}日 {name}",
            day.get_year(),
            day.get_month(),
            day.get_day()
        ),
        format!("{}月{}日 {name}", day.get_month(), day.get_day()),
        name,
    ]
}

struct Cmd {
    /// 引导符后面的码
    key: &'static str,
    /// 给用户看的名字
    label: &'static str,
    build: fn(&Now) -> Vec<String>,
}

const CMDS: &[Cmd] = &[
    Cmd {
        key: "rq",
        label: "日期",
        build: |n| {
            vec![
                n.date_cn(),
                n.date_dash(),
                format!("{:04}/{:02}/{:02}", n.year, n.month, n.day),
                format!("{:04}.{:02}.{:02}", n.year, n.month, n.day),
                format!("{}月{}日", n.month, n.day),
                format!("{}日", n.day),
            ]
        },
    },
    Cmd {
        key: "sj",
        label: "时间",
        build: |n| {
            vec![
                n.time_hm(),
                n.time_hms(),
                format!("{}{}点{:02}分", n.ampm(), n.hour12(), n.minute),
                format!("{}{}:{:02}", n.ampm(), n.hour12(), n.minute),
            ]
        },
    },
    Cmd {
        key: "xq",
        label: "星期",
        build: |n| vec![n.weekday_cn().to_string(), n.weekday_short().to_string()],
    },
    Cmd {
        key: "dt",
        label: "日期时间",
        build: |n| {
            vec![
                format!("{} {}", n.date_dash(), n.time_hms()),
                format!("{} {}", n.date_dash(), n.time_hm()),
                format!("{} {}", n.date_cn(), n.time_hm()),
                format!("{} {} {}", n.date_cn(), n.time_hm(), n.weekday_cn()),
            ]
        },
    },
    Cmd {
        key: "ts",
        label: "时间戳",
        build: |n| vec![n.epoch.to_string(), n.epoch_ms.to_string()],
    },
    Cmd {
        key: "nl",
        label: "农历",
        build: |n| cached(n, |c| &c.lunar),
    },
    Cmd {
        key: "gz",
        label: "干支",
        build: |n| cached(n, |c| &c.ganzhi),
    },
    Cmd {
        key: "jq",
        label: "节气",
        build: |n| cached(n, |c| &c.term),
    },
];

/// 可用命令清单（设置窗/文档里展示用）
pub fn commands() -> Vec<(&'static str, &'static str)> {
    CMDS.iter().map(|c| (c.key, c.label)).collect()
}

/// 给引导符（`PREFIX`，即 `/`）后面的码生成候选。
///
/// - 空串：列出全部命令的当前值预览（按空格拿日期，按数字选别的）
/// - 是某个命令的前缀：展开该命令的所有格式
/// - 匹配不到：空
pub fn candidates(key: &str) -> Vec<String> {
    if !key.chars().all(|c| c.is_ascii_lowercase()) {
        return Vec::new();
    }
    let hits: Vec<&Cmd> = CMDS.iter().filter(|c| c.key.starts_with(key)).collect();
    if hits.is_empty() {
        return Vec::new();
    }

    let now = Now::local();
    if hits.len() == 1 {
        return (hits[0].build)(&now);
    }
    // 多个命令都匹配（只可能是空串）：各取第一项当预览
    hits.iter()
        .filter_map(|c| (c.build)(&now).into_iter().next())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_key_lists_all_commands() {
        let r = candidates("");
        println!("预览: {r:?}");
        for (k, name) in commands() {
            println!("\\{k} {name}: {:?}", candidates(k));
        }
        assert_eq!(r.len(), CMDS.len(), "got {r:?}");
        // 第一项是日期，形如 2026年10月22日
        assert!(r[0].contains('年') && r[0].contains('月') && r[0].contains('日'));
    }

    #[test]
    fn prefix_is_enough() {
        // 首字母互不相同，所以一个字母就能确定命令
        assert_eq!(candidates("r"), candidates("rq"));
        assert_eq!(candidates("s"), candidates("sj"));
        assert_eq!(candidates("x"), candidates("xq"));
        assert_eq!(candidates("d"), candidates("dt"));
        assert_eq!(candidates("t"), candidates("ts"));
        assert_eq!(candidates("n"), candidates("nl"));
        assert_eq!(candidates("g"), candidates("gz"));
        assert_eq!(candidates("j"), candidates("jq"));
    }

    /// 固定时刻。农历 / 干支 / 节气都要能拿确定的输入去测，不能依赖「今天」。
    fn at(y: u16, m: u16, d: u16, h: u16, mi: u16, s: u16) -> Now {
        Now {
            year: y,
            month: m,
            day: d,
            weekday: 0,
            hour: h,
            minute: mi,
            second: s,
            epoch: 0,
            epoch_ms: 0,
        }
    }

    #[test]
    fn lunar_matches_spring_festival() {
        // 2024-02-10 是甲辰年正月初一（春节），公开可查
        let r = lunar_candidates(&at(2024, 2, 10, 12, 0, 0));
        assert_eq!(r[0], "农历甲辰年正月初一");
        assert_eq!(r[1], "甲辰年正月初一");
        assert_eq!(r[2], "正月初一");
        // 前一天是癸卯年十二月三十（除夕）。Tyme 用「十二月」不用「腊月」，
        // 这里跟着库的措辞走，别自己改一套。
        let r = lunar_candidates(&at(2024, 2, 9, 12, 0, 0));
        assert_eq!(r[2], "十二月三十");
    }

    #[test]
    fn lunar_handles_leap_month() {
        // 2023 年闰二月，闰月初一落在 3 月 22 日
        let r = lunar_candidates(&at(2023, 3, 22, 12, 0, 0));
        assert!(r[2].starts_with("闰二月"), "got {r:?}");
    }

    #[test]
    fn ganzhi_year_switches_at_lichun_not_new_year() {
        // 年柱按立春（2024-02-04）换，不按春节（2024-02-10）换
        let before = ganzhi_candidates(&at(2024, 1, 15, 12, 0, 0));
        let after = ganzhi_candidates(&at(2024, 2, 10, 12, 0, 0));
        assert!(before[2].starts_with("癸卯年"), "{before:?}");
        assert!(after[2].starts_with("甲辰年"), "{after:?}");
        // 四柱各柱都是两字，空格分隔的四柱版正好 4 段
        assert_eq!(after[1].split_whitespace().count(), 4, "{after:?}");
        assert!(after[0].ends_with('时'), "{after:?}");
    }

    #[test]
    fn term_is_the_next_one_and_lands_on_a_term_day() {
        let n = at(2026, 10, 23, 12, 0, 0);
        let today = solar_day(&n);
        let (day, name) = next_term_day(&n);
        // 给出的那天本身就是节气日
        assert_eq!(day.get_term_day().get_day_index(), 0, "{name} {day}");
        // 不超过今天 +16 天（一个节气间隔最长约 15.7 天）
        assert!(!today.is_after(day), "must not be in the past: {day}");
        assert!(today.subtract(day) <= 16, "{name} {day}");
        // 三个候选：完整日期 / 月日 / 只有节气名
        let r = term_candidates(&n);
        assert_eq!(r.len(), 3);
        assert_eq!(r[2], name);
        assert!(r[0].ends_with(&name) && r[1].ends_with(&name), "{r:?}");
    }

    #[test]
    fn term_on_a_term_day_returns_today() {
        // 2024-02-04 是立春，当天问「下一个节气」就给今天这个
        let (day, name) = next_term_day(&at(2024, 2, 4, 12, 0, 0));
        assert_eq!(name, "立春");
        assert_eq!((day.get_year(), day.get_month(), day.get_day()), (2024, 2, 4));
    }

    #[test]
    fn tyme_cache_returns_same_result_within_a_minute() {
        let n = at(2026, 10, 23, 12, 30, 0);
        let a = lunar_candidates(&n);
        let b = cached(&n, |c| &c.lunar);
        let c = cached(&at(2026, 10, 23, 12, 30, 59), |c| &c.lunar);
        assert_eq!(a, b);
        assert_eq!(b, c, "同一分钟内（秒变了）应该命中缓存");
        // 换一天必须重算
        let d = cached(&at(2026, 10, 24, 0, 0, 0), |c| &c.lunar);
        assert_ne!(b, d);
    }

    #[test]
    fn date_formats() {
        let r = candidates("rq");
        assert!(r.len() >= 4, "got {r:?}");
        // 第二项是 ISO 风格，形如 2026-10-22
        let iso = &r[1];
        assert_eq!(iso.len(), 10, "expected YYYY-MM-DD, got {iso}");
        assert_eq!(iso.as_bytes()[4], b'-');
        assert_eq!(iso.as_bytes()[7], b'-');
    }

    #[test]
    fn time_has_seconds_and_12h() {
        let r = candidates("sj");
        assert!(r[0].len() == 5, "expected HH:MM, got {}", r[0]);
        assert!(r[1].len() == 8, "expected HH:MM:SS, got {}", r[1]);
        assert!(r.iter().any(|s| s.contains('点')), "expected 中文写法 in {r:?}");
    }

    #[test]
    fn weekday_is_chinese() {
        let r = candidates("xq");
        assert_eq!(r.len(), 2);
        assert!(r[0].starts_with('星') && r[0].ends_with('日') || r[0].starts_with("星期"));
        assert!(r[1].starts_with('周'));
    }

    #[test]
    fn timestamp_is_numeric() {
        let r = candidates("ts");
        assert_eq!(r.len(), 2);
        assert!(r[0].chars().all(|c| c.is_ascii_digit()), "got {}", r[0]);
        assert!(r[1].chars().all(|c| c.is_ascii_digit()), "got {}", r[1]);
        assert!(r[1].len() > r[0].len(), "millis should be longer");
    }

    #[test]
    fn unknown_key_is_empty() {
        assert!(candidates("zz").is_empty());
        assert!(candidates("rqz").is_empty());
        assert!(candidates("1").is_empty());
    }

    #[test]
    fn datetime_contains_both() {
        let r = candidates("dt");
        assert!(!r.is_empty());
        assert!(r[0].contains('-') && r[0].contains(':'), "got {}", r[0]);
    }
}
