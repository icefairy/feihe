//! 手动降权：把某个码下的某个词压到候选最后，但**不删**它。
//!
//! ## 为什么需要它
//!
//! 动态词频只会「越用越靠前」。可有些词是**误选一次就压住一批正经候选**的：
//! 打 `de`，第二位长期是「得」而你想打「的」后面的「地」；打 `shi`，第一页塞满了
//! 你不用的词。这时候需要的不是「多选几次把别的顶上来」（要把每个词都选上一遍），
//! 而是「把这个词按下去」。
//!
//! 置顶是「拉一个上来」，降权是「按一个下去」。两者合起来才算把候选顺序交到用户手上。
//!
//! ## 和置顶的关系
//!
//! 顺序：`置顶 > 一简字 > 二简词 > 自定义短语 > 降权后的词频排序`。
//!
//! 降权在 `freq::rank` 之后立刻执行，所以后面几道「显式提权」都能把它拽回来——
//! 用户真给这个词钉了置顶，或者它本身是一简字，那还是它。**降权压的是「默认排序」，
//! 不是用户的显式选择。**
//!
//! ## 触发方式：鼠标
//!
//! `Ctrl` + 左键点候选（右键是置顶，左键是选词，两者都占了）。不需要记住任何全局
//! 快捷键，也不会和别的软件抢键——这是刻意选的：降权是低频动作，为它占一个全局
//! 组合键不划算（项目里刚因为同样的理由去掉过全角切换的快捷键）。
//!
//! 再 `Ctrl`+左键点一次同一个词 = 取消降权。
//!
//! ## 文件
//!
//! `%APPDATA%\Feihe\demoted.txt`，每行 `编码<TAB>词`：
//!
//! ```text
//! # 飞鹤输入法手动降权
//! # 格式：编码<TAB>词（删掉本文件即全部恢复）
//! de<TAB>得
//! ```
//!
//! 和 `pinned.txt` 一个风格：能看、能手改、能删。删掉整个文件就是全部恢复。
//! 编码是**查词用的那个串**，所以双拼模式和全拼模式下各记各的。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

/// 编码 → 被降权的词集合。一个码下可能按下去好几个。
type Table = HashMap<String, HashSet<Arc<str>>>;

static TABLE: RwLock<Option<Table>> = RwLock::new(None);
/// 上次读盘时文件的修改时间，用来判断别的进程有没有改过
static LAST_MTIME: Mutex<Option<std::time::SystemTime>> = Mutex::new(None);

pub fn file() -> std::path::PathBuf {
    crate::config::dir().join("demoted.txt")
}

/// 查询用的键。和 `pin` 同一个理由：`parse` / `toggle` 都按小写存，查询侧不跟上
/// 就会静默查不中（「降权了但没反应」且不报错）。
///
/// 常见输入本来就是小写，所以先扫一眼，有大写才分配。
fn key_of(code: &str) -> std::borrow::Cow<'_, str> {
    if code.bytes().any(|b| b.is_ascii_uppercase()) {
        std::borrow::Cow::Owned(code.to_ascii_lowercase())
    } else {
        std::borrow::Cow::Borrowed(code)
    }
}

/// 解析 `编码<TAB>词` 文本。抽成纯函数才能单测（真文件读写会碰 APPDATA）。
///
/// 同一个「码 + 词」出现多次只算一条；同一个码下可以有多个词，这是它和
/// `pin`（一个码只留一个词）的区别。
fn parse(text: &str) -> Table {
    let mut out = Table::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((code, word)) = line.split_once('\t') else {
            continue;
        };
        let (code, word) = (code.trim(), word.trim());
        if code.is_empty() || word.is_empty() {
            continue;
        }
        out.entry(code.to_ascii_lowercase())
            .or_default()
            .insert(Arc::from(word));
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

fn with_table<R>(f: impl FnOnce(&mut Table) -> R) -> R {
    ensure_loaded();
    let mut g = match TABLE.write() {
        Ok(g) => g,
        // 锁中毒：当成空表继续跑，绝不能因此崩掉宿主
        Err(e) => e.into_inner(),
    };
    f(g.get_or_insert_with(HashMap::new))
}

/// 和 `freq::rank` 同一类操作：**只重排，不增删**。
///
/// 稳定分区：没被降权的保持原顺序在前，被降权的保持原顺序垫后。抽成独立函数是为了
/// 单测——判据用闭包传进来，测试不需要真文件，也不会碰真实词库（项目约定）。
fn reorder(out: &mut Vec<std::borrow::Cow<'static, str>>, demoted: impl Fn(&str) -> bool) {
    // 一条候选谈不上「排后面」，省掉一次扫描
    if out.len() < 2 {
        return;
    }
    if !out.iter().any(|w| demoted(w.as_ref())) {
        return;
    }
    let mut keep = Vec::with_capacity(out.len());
    let mut tail = Vec::new();
    for w in out.drain(..) {
        if demoted(w.as_ref()) {
            tail.push(w);
        } else {
            keep.push(w);
        }
    }
    out.extend(keep);
    out.extend(tail);
}

/// 把 `code` 下被降权的词压到候选末尾。**在 `freq::rank` 之后立刻调用。**
///
/// 快路径：表里没有这个码、或者候选里一条被降权的都没有 → 一次遍历就返回，
/// 不重新分配。降权是默认没人用的功能，不能因为它让每次按键都多一次 `Vec` 分配。
pub fn push_back(out: &mut Vec<std::borrow::Cow<'static, str>>, code: &str) {
    if out.len() < 2 || code.is_empty() {
        return;
    }
    ensure_loaded();
    let key = key_of(code);
    let g = match TABLE.read() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    let Some(set) = g.as_ref().and_then(|t| t.get(key.as_ref())) else {
        return;
    };
    reorder(out, |w| set.contains(w));
}

/// 切换：这个「码 + 词」已经降权就恢复，否则按下去。返回切换后是否处于降权状态。
///
/// 同一个动作的两面（`Ctrl`+左键点是同一个手势），用一个入口切换，用户不必再记
/// 「怎么取消」。
pub fn toggle(code: &str, word: &str) -> bool {
    if code.is_empty() || word.is_empty() {
        return false;
    }
    let code = code.to_ascii_lowercase();
    let word = Arc::<str>::from(word);
    mutate(|t| {
        let set = t.entry(code.clone()).or_default();
        if set.contains(&word) {
            set.remove(&word);
            // 空集合留在表里等于留一行空记录，顺手清掉——`count()` 和文件都干净
            if set.is_empty() {
                t.remove(&code);
            }
            false
        } else {
            set.insert(word);
            true
        }
    })
}

/// 一共按下去多少条（设置窗显示用）。
pub fn count() -> usize {
    with_table(|t| t.values().map(|s| s.len()).sum())
}

/// 清空全部降权，返回清掉的条数。
pub fn clear() -> usize {
    mutate(|t| {
        let n = t.values().map(|s| s.len()).sum();
        t.clear();
        n
    })
}

/// 所有写操作都走这里：**先读盘、再改、再写盘**。
///
/// 理由和 `pin::mutate` 一样：合并语义（`or_insert`）会让「取消降权」被磁盘旧内容
/// 灌回来，表现为「点了没反应」。表很小（几十行），整读整写不心疼。
fn mutate<R>(f: impl FnOnce(&mut Table) -> R) -> R {
    let disk = load_from_disk();
    let r = with_table(|t| {
        *t = disk;
        f(t)
    });
    write_now();
    r
}

/// 把内存表整份写盘（原子改名）。
fn write_now() {
    // 测试进程绝不写盘（和 config::save 同样的理由）
    if cfg!(test) {
        return;
    }
    let text = with_table(|t| {
        let mut codes: Vec<(&String, &HashSet<Arc<str>>)> = t.iter().collect();
        codes.sort_by(|a, b| a.0.cmp(b.0));
        let mut s = String::with_capacity(t.values().map(|v| v.len()).sum::<usize>() * 20 + 64);
        s.push_str("# 飞鹤输入法手动降权\n# 格式：编码<TAB>词（删掉本文件即全部恢复）\n");
        for (c, words) in codes {
            let mut v: Vec<&str> = words.iter().map(|w| w.as_ref()).collect();
            v.sort_unstable();
            for w in v {
                s.push_str(c);
                s.push('\t');
                s.push_str(w);
                s.push('\n');
            }
        }
        s
    });

    let path = file();
    let _ = crate::config::write_atomic(&path, &text);
    // 记下自己刚写的 mtime，免得紧接着的 refresh() 白读一遍
    if let Some(m) = std::fs::metadata(&path).ok().and_then(|x| x.modified().ok()) {
        if let Ok(mut g) = LAST_MTIME.lock() {
            *g = Some(m);
        }
    }
}

/// 别的进程改过 `demoted.txt` 之后把新内容拉进内存。**在 `Activate` 里调**。
pub fn refresh() {
    let path = file();
    let Some(m) = std::fs::metadata(&path).ok().and_then(|x| x.modified().ok()) else {
        // 文件被删了 → 内存里也不该再留着降权（「删文件即恢复」）
        let nonempty = TABLE
            .read()
            .map(|g| g.as_ref().is_some_and(|t| !t.is_empty()))
            .unwrap_or(false);
        if nonempty {
            with_table(|t| t.clear());
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
    // 整份替换而不是合并：降权是「以文件为准」的表，删掉一行就该恢复
    let disk = load_from_disk();
    with_table(|t| {
        t.clear();
        t.extend(disk);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;

    fn v(words: &[&'static str]) -> Vec<Cow<'static, str>> {
        words.iter().map(|w| Cow::Borrowed(*w)).collect()
    }

    fn names(v: &[Cow<'static, str>]) -> Vec<String> {
        v.iter().map(|w| w.to_string()).collect()
    }

    /// 同一个码下可以按下去好几个词，这是它和置顶（一码一词）的区别。
    #[test]
    fn parses_many_words_per_code() {
        let t = parse("# 注释\nde\t得\nde\t地\nshi\t时\n坏行没有制表符\n\n de \t 德 \n\t空码\n");
        let de = t.get("de").expect("de 应该有两条以上");
        assert_eq!(de.len(), 3, "得 / 地 / 德 都在：{de:?}");
        assert!(de.contains("得"));
        assert!(t.get("shi").is_some_and(|s| s.len() == 1));
        assert!(!t.contains_key(""), "空码不能进表");
    }

    /// 重复行只算一条，写回时不会越写越多。
    #[test]
    fn duplicate_lines_collapse() {
        let t = parse("de\t得\nde\t得\n");
        assert_eq!(t["de"].len(), 1);
    }

    #[test]
    fn code_is_lowercased() {
        let t = parse("DE\t的\n");
        assert!(t.contains_key("de"));
    }

    /// 核心语义：被降权的词沉到最后，其余**保持原顺序**（不能顺手重排，
    /// 排序是 `freq` 的事，这里只负责「压下去」）。
    #[test]
    fn reorder_moves_demoted_to_the_end_stably() {
        let mut out = v(&["甲", "乙", "丙", "丁"]);
        reorder(&mut out, |w| w == "乙");
        assert_eq!(names(&out), ["甲", "丙", "丁", "乙"]);

        let mut out = v(&["甲", "乙", "丙", "丁"]);
        reorder(&mut out, |w| w == "甲" || w == "丙");
        assert_eq!(names(&out), ["乙", "丁", "甲", "丙"], "两个被降权的保持原相对顺序");
    }

    /// 没有一条被降权 → 原样返回（常见路径，不能有任何重排）。
    #[test]
    fn reorder_without_hits_is_a_noop() {
        let mut out = v(&["甲", "乙", "丙"]);
        reorder(&mut out, |_| false);
        assert_eq!(names(&out), ["甲", "乙", "丙"]);
    }

    /// 一条候选谈不上「排后面」。
    #[test]
    fn reorder_ignores_single_candidate() {
        let mut out = v(&["甲"]);
        reorder(&mut out, |_| true);
        assert_eq!(names(&out), ["甲"]);
    }

    /// 词不在候选里就什么都不发生——降权是「只重排、不插入」，
    /// 硬塞一个不在候选里的词等于给用户一个错字。
    #[test]
    fn words_outside_the_list_are_ignored() {
        let mut out = v(&["甲", "乙"]);
        reorder(&mut out, |w| w == "丙");
        assert_eq!(names(&out), ["甲", "乙"]);
    }
}
