//! 动态词频：记住用户**实际选了哪个词**，下次把它往前排。
//!
//! ## 为什么要单独一份表
//!
//! `base.bin` 里的顺序是构建期按 rime-ice 的语料词频排好的，那是「大众习惯」；
//! 但每个人都有自己的高频词——医生打「诊断」、程序员打「函数」，在通用语料里
//! 都排不到前面。静态词频再怎么调也解决不了「我的词」这件事，只能靠学。
//!
//! 学习结果不能写回 `base.bin`：那是 `include_bytes!` 嵌在 DLL `.rdata` 里的
//! 只读数据，多进程共享同一份物理页，改它既不可能也没意义（改了别的进程也看不到）。
//! 所以单独存一份 `%APPDATA%\Feihe\freq.txt`，运行时叠加到查询结果上。
//!
//! ## 只学「用户从候选里选中的词」
//!
//! 触发点在 `textservice` 的两个上屏路径：`commit_with`（选候选 / 空格 / 标点）
//! 和 `commit_segments`（分词逐段选）。回车原样上屏（`commit_raw`）、快捷输入、
//! 软键盘符号都**不学**——那些不是「选词」，学进来只会污染表。
//!
//! ## 排序规则
//!
//! 稳定排序，键是学习次数降序。次数相同就保持词库原有顺序，所以：
//! - 没学过的词，相对顺序完全不变（不会因为引入这个功能把候选顺序搞乱）
//! - 学过的词按次数沉浮，次数一样时仍是「语料词频高的在前」
//!
//! ## 落盘
//!
//! 每次 `bump` 只改内存并记脏，**最多 3 秒写一次盘**。上屏是高频动作，
//! 每次都写文件既没必要也伤盘；反过来只在退出时写又不可靠——TSF 宿主进程
//! 什么时候退出、`DllCanUnloadNow` 会不会被调，都不由我们说了算。
//! 丢最多 3 秒的学习量，可以接受。
//!
//! 文件是纯文本 `词<TAB>次数`，和 `user_dict.txt` 一个风格：能看、能手改、能删。
//! 删掉整个文件就是「重置我的输入习惯」。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

/// 学习表上限。超过就不再收新词（老词照样累加），防止常年使用后无限膨胀。
/// 5 万条约 700KB 内存，对一个输入法进程不算什么。
const MAX_ENTRIES: usize = 50_000;

/// 两次写盘之间的最小间隔
const FLUSH_INTERVAL: Duration = Duration::from_secs(3);

/// 单次选中能贡献的最大次数。用户按住空格连打同一个词时，不至于把它顶到
/// 一个荒谬的高度（也没必要，因为排序只看相对大小）。
const MAX_COUNT: u32 = 1000;

/// 学习表：词 → 被选中的次数
static TABLE: RwLock<Option<HashMap<String, u32>>> = RwLock::new(None);
/// 有未落盘的改动
static DIRTY: AtomicBool = AtomicBool::new(false);
/// 上次写盘时间
static LAST_FLUSH: Mutex<Option<Instant>> = Mutex::new(None);
/// 上次读盘时文件的修改时间（用来判断别的进程有没有写过）
static LAST_MTIME: Mutex<Option<std::time::SystemTime>> = Mutex::new(None);

pub fn file() -> std::path::PathBuf {
    crate::config::dir().join("freq.txt")
}

/// 是否启用。关闭时 `bump` 不记、`rank` 不动，但**已有数据不清空**——
/// 用户只是暂时不想要这个行为，不是要删掉自己的习惯。
fn enabled() -> bool {
    crate::config::get().learn
}

/// 解析 `词<TAB>次数` 文本。抽成纯函数才能单测（真文件读写会碰 APPDATA）。
fn parse(text: &str) -> HashMap<String, u32> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((w, n)) = line.split_once('\t') else {
            continue;
        };
        let (w, n) = (w.trim(), n.trim());
        if w.is_empty() {
            continue;
        }
        // 解析不出来按 1 算：手改坏了也不至于丢掉这个词
        let n: u32 = n.parse().unwrap_or(1).clamp(1, MAX_COUNT);
        out.insert(w.to_string(), n);
    }
    out
}

/// 把磁盘上的表并进内存表：同一个词**取较大值**，不相加。
///
/// 为什么是 max 而不是 +=：两个宿主进程（比如记事本和 Word）各自数到 5 次和 3 次，
/// 它们描述的是同一段历史，真实次数更接近 5 而不是 8。相加会随进程数膨胀。
fn merge_into(t: &mut HashMap<String, u32>, disk: HashMap<String, u32>) {
    for (w, n) in disk {
        match t.get_mut(&w) {
            Some(c) => *c = (*c).max(n),
            None => {
                if t.len() >= MAX_ENTRIES {
                    continue;
                }
                t.insert(w, n);
            }
        }
    }
}

/// 惰性加载：第一次用到才读文件。
fn ensure_loaded() {
    if TABLE.read().map(|g| g.is_none()).unwrap_or(false) {
        let text = std::fs::read_to_string(file()).unwrap_or_default();
        let map = parse(&text);
        if let Ok(mut g) = TABLE.write() {
            if g.is_none() {
                *g = Some(map);
            }
        }
    }
}

fn with_table<R>(f: impl FnOnce(&mut HashMap<String, u32>) -> R) -> R {
    ensure_loaded();
    let mut g = match TABLE.write() {
        Ok(g) => g,
        // 锁中毒（某个线程 panic 了）：当成空表继续跑，绝不能因此崩掉宿主
        Err(e) => e.into_inner(),
    };
    f(g.get_or_insert_with(HashMap::new))
}

/// 记一次「用户选了这个词」。
pub fn bump(word: &str) {
    if word.is_empty() || !enabled() {
        return;
    }
    // 纯 ASCII 一律不学，两种情况都会走到这里：
    // 1. 组合态打标点时 `commit_with` 会带上候选（「，」），那是标点不是选词
    // 2. 一个候选都没有时 `current_word()` 退化成原始双拼码（`nihao`），那不是词
    // 中文字词一定含非 ASCII 字符，这条判据足够准。
    if word.is_ascii() {
        return;
    }
    let word = word.to_string();
    with_table(|t| {
        match t.get_mut(&word) {
            Some(n) => *n = (*n + 1).min(MAX_COUNT),
            None => {
                if t.len() >= MAX_ENTRIES {
                    return;
                }
                t.insert(word, 1);
            }
        }
    });
    DIRTY.store(true, Ordering::Release);
    maybe_flush();
}

/// 到点就写盘。调用频率是「每次上屏一次」，所以这里只做两次原子读 + 一次比较。
fn maybe_flush() {
    let due = {
        let mut g = match LAST_FLUSH.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        let due = match *g {
            Some(t) => t.elapsed() >= FLUSH_INTERVAL,
            None => true,
        };
        if due {
            *g = Some(Instant::now());
        }
        due
    };
    if due {
        flush();
    }
}

/// 立即写盘。写失败只记日志——学习表是锦上添花，不能因为它影响打字。
pub fn flush() {
    if !DIRTY.swap(false, Ordering::AcqRel) {
        return;
    }
    // 测试进程绝不写盘（和 config::save 同样的理由）
    if cfg!(test) {
        return;
    }
    // **写之前先把磁盘上的并进来。** 每个宿主进程各有一份内存表，
    // 直接覆盖会让「在记事本里学的词」被「在 Word 里学的词」冲掉——
    // 表现是「我明明用了很多次的词，怎么又沉下去了」，而且完全无法复现。
    let disk = parse(&std::fs::read_to_string(file()).unwrap_or_default());
    let text = with_table(|t| {
        merge_into(t, disk);
        let mut v: Vec<(&String, &u32)> = t.iter().collect();
        // 次数高的在前，人看起来更像「我的常用词」
        v.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        let mut s = String::with_capacity(v.len() * 16 + 32);
        s.push_str("# 飞鹤输入法动态词频\n# 格式：词<TAB>次数（删掉本文件即重置）\n");
        for (w, n) in v {
            s.push_str(w);
            s.push('\t');
            s.push_str(&n.to_string());
            s.push('\n');
        }
        s
    });
    let _ = std::fs::create_dir_all(crate::config::dir());
    let path = file();
    let tmp = path.with_extension("txt.tmp");
    if std::fs::write(&tmp, text).is_ok() {
        if std::fs::rename(&tmp, &path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
    // 记下自己刚写的 mtime，免得紧接着的 refresh() 白读一遍
    if let Some(m) = std::fs::metadata(&path).ok().and_then(|x| x.modified().ok()) {
        if let Ok(mut g) = LAST_MTIME.lock() {
            *g = Some(m);
        }
    }
}

/// 把别的进程学到的词拉进内存。**在 `Activate` 里调**（切输入法、切窗口时）。
///
/// 不做这件事的话，每个宿主进程只看得见自己学过的东西：在 Word 里把「诊断」
/// 选到第一位，切到浏览器里它又沉回去了。用户会觉得「这功能时灵时不灵」。
///
/// 只在文件 mtime 变了的时候才真读盘——`Activate` 是高频动作，
/// 不能每次都解析一遍几千行的文本。
pub fn refresh() {
    // 先把自己的改动写下去（flush 内部会合并磁盘），否则下面的读盘会看不到它们
    flush();

    let path = file();
    let Some(m) = std::fs::metadata(&path).ok().and_then(|x| x.modified().ok()) else {
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
    let disk = parse(&std::fs::read_to_string(&path).unwrap_or_default());
    if disk.is_empty() {
        return;
    }
    with_table(|t| merge_into(t, disk));
}

/// 按学习次数稳定重排候选。次数相同的保持原有相对顺序。
pub fn rank(words: &mut [std::borrow::Cow<'static, str>]) {
    if words.len() < 2 || !enabled() {
        return;
    }
    // 直接借读锁里的表，**不克隆**：这个函数每次按键都会跑，
    // 克隆一张 5 万条的表是灾难。排序期间持读锁，写锁只在 bump 时短暂争用。
    ensure_loaded();
    let g = match TABLE.read() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    let Some(table) = g.as_ref() else { return };
    if table.is_empty() {
        return;
    }
    // 稳定排序：`sort_by_key` 保证相等元素保持原顺序，这是「不搞乱语料顺序」的前提
    words.sort_by_key(|w| std::cmp::Reverse(table.get(w.as_ref()).copied().unwrap_or(0)));
}

/// 清空学习表（设置窗「重置」用）。返回清掉的条目数。
pub fn clear() -> usize {
    let n = with_table(|t| {
        let n = t.len();
        t.clear();
        n
    });
    DIRTY.store(true, Ordering::Release);
    flush();
    n
}

/// 学习表条目数（设置窗显示用）
pub fn len() -> usize {
    with_table(|t| t.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;

    #[test]
    fn parses_counts_and_ignores_junk() {
        let t = parse("# 注释\n你好\t7\n坏行没有制表符\n\n 世界 \t 3 \n空词\t5\n\t9\n");
        assert_eq!(t.get("你好"), Some(&7));
        assert_eq!(t.get("世界"), Some(&3));
        assert_eq!(t.get("空词"), Some(&5));
        assert_eq!(t.len(), 3, "没有制表符的行、空词都要跳过");
    }

    #[test]
    fn bad_count_falls_back_to_one() {
        // 手改成乱七八糟的值时，宁可当成 1 次也不丢这个词
        let t = parse("词\tabc\n另一个\t-5\n");
        assert_eq!(t.get("词"), Some(&1));
        assert_eq!(t.get("另一个"), Some(&1));
    }

    #[test]
    fn count_is_clamped() {
        let t = parse("词\t999999\n");
        assert_eq!(t.get("词"), Some(&MAX_COUNT));
    }

    /// 排序必须是**稳定**的：没学过的词一个都不能动。
    /// 这是这个功能最容易出事的地方——一次不小心用 sort_unstable，
    /// 所有码的候选顺序都会变，用户会立刻觉得「输入法乱套了」。
    #[test]
    fn rank_keeps_unlearned_order() {
        let mut v = vec![
            Cow::Borrowed("甲"),
            Cow::Borrowed("乙"),
            Cow::Borrowed("丙"),
        ];
        // 表里没有任何词 → 顺序原样
        rank(&mut v);
        assert_eq!(v, vec!["甲", "乙", "丙"]);
    }

    /// 直接验证排序语义（不走全局表，避免测试之间互相干扰）
    #[test]
    fn stable_sort_by_count_moves_learned_to_front() {
        let mut table: HashMap<String, u32> = HashMap::new();
        table.insert("丙".into(), 5);
        let mut v = vec![
            Cow::Borrowed("甲"),
            Cow::Borrowed("乙"),
            Cow::Borrowed("丙"),
        ];
        v.sort_by_key(|w| std::cmp::Reverse(table.get(w.as_ref()).copied().unwrap_or(0)));
        assert_eq!(v, vec!["丙", "甲", "乙"]);
    }

    #[test]
    fn merge_takes_max_not_sum() {
        // 两个进程各数到 5 和 3，描述的是同一段历史，取 5 而不是 8
        let mut mem: HashMap<String, u32> = HashMap::new();
        mem.insert("诊断".into(), 5);
        let mut disk: HashMap<String, u32> = HashMap::new();
        disk.insert("诊断".into(), 3);
        disk.insert("函数".into(), 2);
        merge_into(&mut mem, disk);
        assert_eq!(mem.get("诊断"), Some(&5), "同词取较大值，不能相加");
        assert_eq!(mem.get("函数"), Some(&2), "磁盘独有的词要并进来");
    }

    #[test]
    fn merge_keeps_memory_when_it_is_ahead() {
        let mut mem: HashMap<String, u32> = HashMap::new();
        mem.insert("诊断".into(), 9);
        let mut disk: HashMap<String, u32> = HashMap::new();
        disk.insert("诊断".into(), 4);
        merge_into(&mut mem, disk);
        assert_eq!(mem.get("诊断"), Some(&9), "内存更新时不能被磁盘的旧值拉低");
    }

    #[test]
    fn merge_respects_entry_cap() {
        let mut mem: HashMap<String, u32> = HashMap::new();
        for i in 0..MAX_ENTRIES {
            mem.insert(format!("w{i}"), 1);
        }
        let mut disk: HashMap<String, u32> = HashMap::new();
        disk.insert("新词".into(), 1);
        merge_into(&mut mem, disk);
        assert_eq!(mem.len(), MAX_ENTRIES, "到上限后不再收新词");
        assert!(!mem.contains_key("新词"));
    }

    #[test]
    fn ascii_is_not_learned() {
        // 判据是「纯 ASCII 不学」，挡的是这种情况：一个候选都没有时
        // `current_word()` 会退化成原始双拼码（`nihao`），那不是词。
        for s in ["nihao", "a", "  ", "!!"] {
            assert!(s.is_ascii(), "{s} 应该被判为不该学");
        }
        assert!(!"你好".is_ascii());
        // 注意：全角标点**不是** ASCII，这条判据挡不住它。标点之所以不会被学，
        // 靠的是调用约定——`commit_with` 传进来的是「选中的候选词」，
        // 标点只是拼在后面的 suffix，从来不单独进 bump。
        assert!(!"，".is_ascii());
    }
}
