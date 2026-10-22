//! 快捷输入：日期 / 时间 / 星期 / 时间戳
//!
//! ## 为什么要有引导符
//!
//! 搜狗是**全拼**，所以 `rq`、`sj` 这种两字母码不会撞车。飞鹤是**双拼**，两字母
//! 恰好就是一个音节：`sj` = `s`+`an` = `san`（三），`xq` = `x`+`iu` = `xiu`（休）。
//! 直接照搬会有一半命令被词库吃掉，行为不可预测。
//!
//! 所以统一用 `\` 引导：`\rq` 日期、`\sj` 时间。`\` 不属于任何双拼方案，也不会
//! 出现在全拼音节里，永远不会有歧义。
//!
//! ## 前缀匹配
//!
//! 五个命令的首字母互不相同（r / s / x / d / t），所以 `\r` 就能唯一确定「日期」。
//! 打 `\rq` 和 `\r` 效果一样，不要求用户记全。
//!
//! 只打一个 `\` 时列出全部命令的**当前值预览**——按空格得到最常用的日期，按数字选别的。
//! 这样不用记码也能用。

use windows::Win32::System::SystemInformation::GetLocalTime;

/// 引导符。见模块文档：双拼两字母会和音节撞车，必须有引导符。
pub const PREFIX: char = '\\';

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
];

/// 可用命令清单（设置窗/文档里展示用）
pub fn commands() -> Vec<(&'static str, &'static str)> {
    CMDS.iter().map(|c| (c.key, c.label)).collect()
}

/// 给 `\` 后面的码生成候选。
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
