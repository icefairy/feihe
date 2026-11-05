//! 自定义短语：手写的「编码 → 文本」，支持时间变量。
//!
//! ## 和自造词有什么不同
//!
//! `phrase`（自造词）是**学来的**：分词上屏一次就记一条，用户没主动做过什么。
//! 这里的东西是**写下来的**：用户在 `custom.txt` 里明明白白写着
//! `dz<TAB>北京市朝阳区`，打 `dz` 就该出它，不需要「先选几次让它学会」。
//!
//! 所以两者的排序位置不同：自造词和词库候选一起按词频排（见 `dict::search` 的
//! 注释），自定义短语直接顶到候选最前面。
//!
//! ## 文件
//!
//! `%APPDATA%\Feihe\custom.txt`，每行 `编码<TAB>文本`：
//!
//! ```text
//! # 飞鹤输入法自定义短语
//! dz<TAB>北京市朝阳区{year}年{month}月{day}日
//! sfz<TAB>110101199001011234
//! qm<TAB>此致
//! ```
//!
//! 编码是**查词用的那个串**（双拼是双拼码，全拼模式是全拼串），1–32 个小写字母。
//! 同一个编码可以写多行，按文件顺序排在候选最前。
//!
//! ## 时间变量
//!
//! `{year}` `{month}` `{day}` `{hour}` `{minute}` `{second}` `{weekday}`。
//! 前四个不补零（`9` 而不是 `09`），分秒补两位（`09`），`{weekday}` 是「一」到「日」
//! ——照「读起来是什么」定，不是照「排序方便」定。`{{` `}}` 输出花括号本身，
//! 认不出的 `{…}` 原样留在文本里（宁可显示原文，也不要悄悄吃掉用户的字符）。
//!
//! **展开发生在查询时**，不是加载时：`{day}` 的意义是「今天」，缓存住就跨天错一天。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};


/// 编码 → 这个编码下的若干条文本（按文件顺序）。
type Table = HashMap<String, Vec<Arc<str>>>;

static TABLE: RwLock<Option<Table>> = RwLock::new(None);
/// 上次读盘时文件的修改时间，用来判断用户有没有改过
static LAST_MTIME: Mutex<Option<std::time::SystemTime>> = Mutex::new(None);

/// 编码最长多少个字符。和 `sogou` 那边一个量级：再长就不是「快捷输入」而是「打文章」了。
pub const MAX_CODE_LEN: usize = 32;

pub fn file() -> std::path::PathBuf {
    crate::config::dir().join("custom.txt")
}

/// 解析 `编码<TAB>文本`。抽成纯函数才能单测（真文件读写会碰 APPDATA）。
fn parse(text: &str) -> Table {
    let mut out = Table::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((code, body)) = line.split_once('\t') else {
            continue;
        };
        let (code, body) = (code.trim(), body.trim());
        // 编码只收小写字母：数字和标点在双拼里根本打不出来，
        // 收了也是永远命不中的死条目
        if code.is_empty()
            || code.len() > MAX_CODE_LEN
            || !code.bytes().all(|b| b.is_ascii_lowercase())
        {
            continue;
        }
        if body.is_empty() {
            continue;
        }
        out.entry(code.to_string())
            .or_default()
            .push(Arc::from(body));
    }
    out
}

fn load_from_disk() -> Table {
    parse(&std::fs::read_to_string(file()).unwrap_or_default())
}

fn ensure_loaded() {
    if TABLE.read().map(|g| g.is_none()).unwrap_or(false) {
        let map = load_from_disk();
        if let Ok(mut g) = TABLE.write() {
            if g.is_none() {
                *g = Some(map);
            }
        }
    }
}

/// 这个编码下的短语，时间变量已经展开。没有就是空。
///
/// **入口处统一小写化**（和 `pin::word_for` 同样的理由：不变量不能靠调用链远端维持）。
pub fn words_for(code: &str) -> Vec<String> {
    if code.is_empty() {
        return Vec::new();
    }
    ensure_loaded();
    let owned;
    let key = if code.bytes().any(|b| b.is_ascii_uppercase()) {
        owned = code.to_ascii_lowercase();
        &owned
    } else {
        code
    };
    let g = match TABLE.read() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    let Some(list) = g.as_ref().and_then(|t| t.get(key)) else {
        return Vec::new();
    };
    list.iter().map(|t| expand(t)).collect()
}

/// 别的进程（或用户拿编辑器）改过 `custom.txt` 之后把新内容拉进来。**在 `Activate` 里调**。
pub fn refresh() {
    let path = file();
    let Some(m) = std::fs::metadata(&path).ok().and_then(|x| x.modified().ok()) else {
        // 文件没了 → 内存里也不该留着。删掉文件就是「全部取消」，和 `pinned.txt` 一致。
        let nonempty = TABLE
            .read()
            .map(|g| g.as_ref().is_some_and(|t| !t.is_empty()))
            .unwrap_or(false);
        if nonempty {
            if let Ok(mut g) = TABLE.write() {
                g.get_or_insert_with(Table::new).clear();
            }
        }
        return;
    };
    {
        let mut g = match LAST_MTIME.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        if *g == Some(m) {
            return;
        }
        *g = Some(m);
    }
    let disk = load_from_disk();
    if let Ok(mut g) = TABLE.write() {
        // 整份替换：以文件为准，删掉一行就该消失
        *g = Some(disk);
    }
}

/// 按当时的本地时间展开 `{year}` 这类变量。
///
/// 不认识的 `{xxx}` **原样留着**——用户可能在写别的东西（模板、代码片段），
/// 悄悄吃掉几个字符是最难查的那类 bug。
pub fn expand(text: &str) -> String {
    if !text.contains('{') && !text.contains('}') {
        return text.to_string();
    }
    let now = crate::clock::now();
    let mut out = String::with_capacity(text.len() + 16);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                out.push('}');
            }
            '{' => {
                let mut name = String::new();
                let mut closed = false;
                for c2 in chars.by_ref() {
                    if c2 == '}' {
                        closed = true;
                        break;
                    }
                    name.push(c2);
                }
                match variable(&name, &now) {
                    Some(v) if closed => out.push_str(&v),
                    // 认不出 / 没闭合：原样吐回去
                    _ => {
                        out.push('{');
                        out.push_str(&name);
                        if closed {
                            out.push('}');
                        }
                    }
                }
            }
            c => out.push(c),
        }
    }
    out
}

fn variable(name: &str, now: &crate::clock::LocalTime) -> Option<String> {
    Some(match name {
        "year" => now.year.to_string(),
        "month" => now.month.to_string(),
        "day" => now.day.to_string(),
        "hour" => now.hour.to_string(),
        // 分秒补两位：「9 时 05 分」读起来才对，`{minute}` 单独用时也是
        "minute" => format!("{:02}", now.minute),
        "second" => format!("{:02}", now.second),
        // weekday：0 = 周日
        "weekday" => ["日", "一", "二", "三", "四", "五", "六"]
            .get(now.weekday as usize)?
            .to_string(),
        _ => return None,
    })
}

/// 文件不存在时写一份带注释的模板，让用户知道格式。
///
/// 只在用户主动点「编辑自定义短语」时调——不能每次启动都写，否则用户删掉文件
/// 想清空配置，一重启又长回来了。
pub fn ensure_template() {
    let path = file();
    if path.exists() {
        return;
    }
    let _ = std::fs::create_dir_all(crate::config::dir());
    let _ = crate::config::write_atomic(
        &path,
        "# 飞鹤输入法自定义短语\n\
         # 格式：编码<TAB>文本（删掉本文件即全部取消）\n\
         # 编码：1–32 个小写字母，就是平时打的码\n\
         # 文本里可以用 {year} {month} {day} {hour} {minute} {second} {weekday}\n\
         #\n\
         # 例：\n\
         # dz\t北京市朝阳区{year}年{month}月{day}日\n\
         # sfz\t110101199001011234\n",
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pairs_and_skips_junk() {
        let t = parse(
            "# 注释\n\
             dz\t北京市朝阳区\n\
             sfz\t110101199001011234\n\
             坏行没有制表符\n\
             \n\
             \t空码\n\
             abc\t\n\
             AB\t大写码不收\n\
             dz\t第二个同码短语\n",
        );
        assert_eq!(t.len(), 2, "只该有 dz 和 sfz：{t:?}");
        // 同码多行按文件顺序保留
        let dz: Vec<&str> = t["dz"].iter().map(|s| &**s).collect();
        assert_eq!(dz, ["北京市朝阳区", "第二个同码短语"]);
        assert!(t.contains_key("sfz"));
    }

    #[test]
    fn code_length_is_capped() {
        let long = "a".repeat(MAX_CODE_LEN + 1);
        assert!(parse(&format!("{long}\t太长\n")).is_empty());
        let ok = "a".repeat(MAX_CODE_LEN);
        assert_eq!(parse(&format!("{ok}\t刚好\n")).len(), 1);
    }

    /// 时间变量按「读起来是什么」补零：时分秒定长，年月日不定长。
    #[test]
    fn variables_expand_by_reading() {
        let now = crate::clock::LocalTime {
            year: 2026,
            month: 11,
            day: 4,
            weekday: 3, // 周三
            hour: 9,
            minute: 5,
            second: 7,
        };
        assert_eq!(variable("year", &now).as_deref(), Some("2026"));
        assert_eq!(variable("month", &now).as_deref(), Some("11"));
        assert_eq!(variable("day", &now).as_deref(), Some("4"), "不补零");
        assert_eq!(variable("hour", &now).as_deref(), Some("9"));
        assert_eq!(variable("minute", &now).as_deref(), Some("05"), "补两位");
        assert_eq!(variable("second", &now).as_deref(), Some("07"));
        assert_eq!(variable("weekday", &now).as_deref(), Some("三"));
        assert_eq!(variable("nope", &now), None);
    }

    #[test]
    fn unknown_braces_are_kept_verbatim() {
        // 认不出的变量原样留着——用户可能在写模板，悄悄吃字符最难查
        assert_eq!(expand("a{b}c"), "a{b}c");
        assert_eq!(expand("{nope}"), "{nope}");
        // 没闭合也原样
        assert_eq!(expand("x{year"), "x{year");
        // 转义
        assert_eq!(expand("{{year}}"), "{year}");
        assert_eq!(expand("{{}}"), "{}");
        assert_eq!(expand("}}"), "}");
        // 没有花括号的文本原样返回，一个字节都不动
        assert_eq!(expand("纯文本"), "纯文本");
    }

    #[test]
    fn weekday_covers_all_seven() {
        for (n, want) in ["日", "一", "二", "三", "四", "五", "六"].iter().enumerate() {
            let now = crate::clock::LocalTime {
                weekday: n as u16,
                ..Default::default()
            };
            assert_eq!(variable("weekday", &now).as_deref(), Some(*want));
        }
    }
}
