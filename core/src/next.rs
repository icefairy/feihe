//! 上屏后的联想：记住「这个词后面通常跟什么」，下次打完它就把后续词摆出来。
//!
//! ## 两张表：内置 + 用户
//!
//! - **内置表**（`next_builtin.txt` 在构建期编成 `next.bin`，零解析嵌进 DLL）：从
//!   rime-ice 词库的复合词反推出来的词对（`北京时间` → `北京→时间`），约 7000 个头词 /
//!   2 万条。解决「刚装完一个字都联想不出来」的冷启动。
//! - **用户表**（`next.txt`，数据目录）：用户自己打出来的词对，按选择次数排。
//!
//! 查询时**用户表优先、内置表补位**：两张表的权重不是一回事（内置是词库词频，
//! 用户是选择次数），混在一起排序会让内置表压死用户刚学的那条。
//!
//! ## 为什么是「词对表」而不是语言模型
//!
//! 联想要回答的只有一个问题：**给定上一个词，最可能的后续词是哪几个**。这是一次
//! 查表，不是一次推理。开源的中文 n-gram / 神经模型动辄 4.5 MB 到 420 MB（见
//! `handoff.md` 里那张体积表），换来的能力是「在若干同音读法里挑一个」——那是
//! 选词消歧，不是联想。把模型塞进一个 1.8 MB 的安装包去做查表，是拿卡车送快递。
//!
//! 内置表也不需要语料：`tools/gen_next.py` 把词库里的复合词按最短前缀切开，
//! 就得到了「北京→大学 / 时间 / 地区」这种搭配——这本来就是词库统计出来的答案。
//!
//! ## 只认汉字词
//!
//! 记录前过 `is_word`：全部是汉字、长度 1..=8。标点、字母串、数字串都不记——
//! 「你好，」不是词，「nihao」也不是。上屏带标点时由调用方只取**开头的连续汉字**
//! （`leading_word`），标点本身不参与配对，但也不打断配对链：打完「他说：」，
//! 下一个词仍然和「他说」配对。
//!
//! ## 任何长度的词都要能联想
//!
//! 最早内置表只收 2 字头词（1 字头的「北京→市」是噪音，3 字头又太少），结果就是
//! **只有打完两字词才看得见联想**。真实写作里单字（`我`）、三字词（`计算机`）、
//! 四字词（`上市公司`）都是常客，没有联想等于这个功能只覆盖了一半场景。
//!
//! 现在有两条路一起走：
//!
//! 1. **表里就有**（`tools/gen_next.py` 按头词长度分桶，见那个脚本的注释）。
//! 2. **后缀回退**（[`resolve`]）：`今天北京` / `去北京` / `大北京` 在表里都查不到，
//!    但它们的后缀 `北京` 查得到，而「北京后面接什么」对这三者基本是一回事。
//!    取**最长的已知后缀**，比整个词失配就放弃要有用得多。
//!
//! 后缀最少留 2 个字：`计算机` 落到单字「机」上会联想出一堆和整词无关的碎片，
//! 而「机」后面的词也未必接得上「计算机」。单字头只在**用户真的上屏了一个字**时
//! 才用（`我` → `觉得`），不是从长词里截出来的。
//!
//! ## 容量
//!
//! 头词最多 `MAX_HEADS` 个、每个只留 `MAX_TAIL` 个后续。这不是省内存（整表撑死
//! 几百 KB），是**保质量**：一张什么都记的表里，每个头词都挂着十来个「只出现过
//! 一次」的后续，联想出来全是噪音。留 top-N 等于强迫它只记最靠谱的那几个。
//!
//! 满了淘汰最弱的头词（按该头词所有后续次数之和），一次砍 1/8 留余量——和
//! `freq` 的做法一致，避免「每来一个新词就全表排一遍」。
//!
//! ## 落盘
//!
//! 攒脏、最多 5 秒写一次（`freq` 是 3 秒；联想的价值密度比词频低，可以更懒）。
//! 写失败保留脏标记下次重试——`next.txt` 是用户可能拿编辑器开着的文件，
//! rename 失败是常态路径，不是异常。
//!
//! ## 为什么不合并磁盘
//!
//! `freq` 写盘前要把磁盘内容并回来，因为多个宿主进程各有一份内存表，直接覆盖会
//! 互相冲掉学习结果。这里**不合并**：整表重写，谁最后写谁说了算。理由是词对表
//! 的价值远低于词频表（丢几条联想没人察觉），而合并逻辑会引入「删了又回来」这类
//! 麻烦（见 `freq` 里 `PENDING_CLEAR` 那一整段）。

use std::collections::HashMap;
use std::path::PathBuf;

use crate::nextbin::Bin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

/// 最多记多少个头词。4000 × 6 ≈ 2.4 万条，文件约 400 KB。
const MAX_HEADS: usize = 4000;

/// 一个头词最多挂几个后续。超过就不再收新的（除非新的顶掉最弱的）。
const MAX_TAIL: usize = 6;

/// 联想一次最多给几个候选。
pub const SHOW: usize = 5;

/// 两次写盘之间的最小间隔。
const FLUSH_INTERVAL: Duration = Duration::from_secs(5);

/// 单条次数封顶。联想的排序只看相对大小，不需要精确计数。
///
/// **只用于用户表**（每次上屏 +1）。内置表的权重是生成器算的词库统计，
/// 量级完全不同（最大 19260817），不能套这个上限——把它们统一压到 500 之后
/// 所有词权重相等，排序退化成按字符串，屏幕上就是「三国」后面先出「时代」
/// 而不是「群英传」。这条坑踩过一次，见 `builtin_weights_are_not_flattened`。
const MAX_COUNT: u32 = 500;

/// 淘汰后保留的比例（同 `freq`）。
const EVICT_KEEP_NUM: usize = 7;
const EVICT_KEEP_DEN: usize = 8;

/// `头词 -> [(后续词, 次数)]`，内层按次数降序、长度 ≤ `MAX_TAIL`。
type Table = HashMap<String, Vec<(String, u32)>>;

static TABLE: RwLock<Option<Table>> = RwLock::new(None);
static DIRTY: AtomicBool = AtomicBool::new(false);
static LAST_FLUSH: Mutex<Option<Instant>> = Mutex::new(None);
/// 上次读盘时 `next.txt` 的 mtime。`refresh` 靠它避免每次激活都读一遍
/// （表大了有几百 KB，而「切窗口」是分钟级的动作）。
static LAST_MTIME: Mutex<Option<std::time::SystemTime>> = Mutex::new(None);

/// 内置联想表：构建期编好的扁平二进制（`next.bin`），**零解析**。
///
/// 数据落在只读段，由加载器 mmap，多个宿主进程共享同一份物理页。曾经是
/// `include_str!` + 第一次查询时解析成 HashMap：309 KB 文本 → 5.8 ms / 2.1 MB 堆，
/// 而且那 2.1 MB 是每个进程各一份。格式和取舍见 `nextbin` 的模块注释。
static BUILTIN: Bin = Bin::new(include_bytes!(concat!(env!("OUT_DIR"), "/next.bin")));

fn builtin() -> &'static Bin {
    &BUILTIN
}

fn file() -> PathBuf {
    crate::config::dir().join("next.txt")
}

/// 这个词能不能进表：1..=8 个汉字，别的一律不要。
///
/// 上限 8 是「再长就不像词了」的实用界限；标点、数字、字母都会被拒。
pub fn is_word(s: &str) -> bool {
    let mut n = 0;
    for c in s.chars() {
        if !is_han(c) {
            return false;
        }
        n += 1;
        if n > 8 {
            return false;
        }
    }
    n > 0
}

fn is_han(c: char) -> bool {
    matches!(c,
        '\u{4e00}'..='\u{9fff}'      // CJK 统一表意文字
        | '\u{3400}'..='\u{4dbf}'    // 扩展 A
        | '\u{f900}'..='\u{faff}'    // 兼容表意文字
    )
}

/// 取文本**开头的连续汉字**。
///
/// 上屏的常常是「词 + 标点」（`commit_with` 会把全角标点粘在后面），配对要的是
/// 词本身。返回空串表示这段文本不以汉字开头（纯标点 / 字母 / 数字），
/// 调用方据此决定「不记、也不打断联想链」。
pub fn leading_word(text: &str) -> &str {
    let mut end = 0;
    for (i, c) in text.char_indices() {
        if !is_han(c) {
            break;
        }
        end = i + c.len_utf8();
    }
    &text[..end]
}

fn with_table<R>(f: impl FnOnce(&mut Table) -> R) -> R {
    let mut g = TABLE.write().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = Some(parse(&std::fs::read_to_string(file()).unwrap_or_default()));
    }
    f(g.as_mut().expect("刚填过"))
}

fn parse(text: &str) -> Table {
    let mut t = Table::new();
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut it = line.split('\t');
        let (Some(prev), Some(next), n) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        let Ok(n) = n.unwrap_or("1").trim().parse::<u32>() else {
            continue;
        };
        if !is_word(prev) || !is_word(next) || n == 0 {
            continue;
        }
        // 同一个头词可能会在文件里出现多行（用户手改、或两次写盘交错），并起来
        let tail = t.entry(prev.to_string()).or_default();
        match tail.iter_mut().find(|(w, _)| w == next) {
            Some((_, c)) => *c = (*c).saturating_add(n).min(MAX_COUNT),
            None => tail.push((next.to_string(), n.min(MAX_COUNT))),
        }
    }
    for tail in t.values_mut() {
        tail.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        tail.truncate(MAX_TAIL);
    }
    t
}

fn render(t: &Table) -> String {
    let mut rows: Vec<(&String, &String, u32)> = Vec::new();
    for (prev, tail) in t {
        for (next, n) in tail {
            rows.push((prev, next, *n));
        }
    }
    // 头词按「该头词的总次数」排，人看着像一张常用词表
    let weight: HashMap<&str, u32> = t
        .iter()
        .map(|(k, v)| (k.as_str(), v.iter().map(|(_, n)| *n).sum()))
        .collect();
    rows.sort_by(|a, b| {
        weight[b.0.as_str()]
            .cmp(&weight[a.0.as_str()])
            .then_with(|| a.0.cmp(b.0))
            .then_with(|| b.2.cmp(&a.2))
            .then_with(|| a.1.cmp(b.1))
    });
    let mut s = String::with_capacity(rows.len() * 16 + 128);
    s.push_str(
        "# 飞鹤输入法联想词对\n\
         # 格式：上一个词<TAB>下一个词<TAB>次数\n\
         # 删掉本文件即清空（重启输入法生效）\n",
    );
    for (prev, next, n) in rows {
        s.push_str(prev);
        s.push('\t');
        s.push_str(next);
        s.push('\t');
        s.push_str(&n.to_string());
        s.push('\n');
    }
    s
}

/// 往表里塞一条。纯逻辑，单测直接构造 `Table` 调它。
fn add(t: &mut Table, prev: &str, next: &str) {
    if !is_word(prev) || !is_word(next) {
        return;
    }
    if let Some(tail) = t.get_mut(prev) {
        if let Some((_, n)) = tail.iter_mut().find(|(w, _)| w == next) {
            *n = n.saturating_add(1).min(MAX_COUNT);
        } else if tail.len() < MAX_TAIL {
            tail.push((next.to_string(), 1));
        } else {
            // 满了：和里面最弱的比，新词只有 1 次，永远比不过，直接丢。
            // （「用新词挤掉旧词」会让表在冷启动期来回抖，见模块注释。）
        }
    } else {
        if t.len() >= MAX_HEADS {
            evict(t);
        }
        t.insert(prev.to_string(), vec![(next.to_string(), 1)]);
    }
    // 只有同一个头词内的顺序要维护，小 Vec 排一下无所谓
    if let Some(tail) = t.get_mut(prev) {
        tail.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    }
}

/// 按「头词总次数」淘汰最弱的一批。
fn evict(t: &mut Table) {
    let keep = MAX_HEADS * EVICT_KEEP_NUM / EVICT_KEEP_DEN;
    let mut keys: Vec<(u32, String)> = t
        .iter()
        .map(|(k, v)| (v.iter().map(|(_, n)| *n).sum::<u32>(), k.clone()))
        .collect();
    keys.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    keys.truncate(keep);
    let alive: std::collections::HashSet<String> = keys.into_iter().map(|(_, k)| k).collect();
    t.retain(|k, _| alive.contains(k));
}

fn maybe_flush() {
    let due = {
        let mut g = LAST_FLUSH.lock().unwrap_or_else(|e| e.into_inner());
        match *g {
            None => {
                *g = Some(Instant::now());
                false
            }
            Some(at) if at.elapsed() >= FLUSH_INTERVAL => {
                *g = Some(Instant::now());
                true
            }
            Some(_) => false,
        }
    };
    if due {
        flush();
    }
}

/// 立即写盘。写失败只记脏下次重试——联想是锦上添花，不能因为它影响打字。
pub fn flush() {
    if !DIRTY.swap(false, Ordering::AcqRel) {
        return;
    }
    if cfg!(test) {
        return;
    }
    let text = with_table(|t| render(t));
    if crate::config::write_atomic(&file(), &text).is_err() {
        DIRTY.store(true, Ordering::Release);
        return;
    }
    // 记下自己刚写的 mtime，免得紧接着的 `refresh()` 白读一遍
    if let Some(m) = std::fs::metadata(file()).ok().and_then(|x| x.modified().ok()) {
        if let Ok(mut g) = LAST_MTIME.lock() {
            *g = Some(m);
        }
    }
}

/// 记一对词：上次上屏 `prev`，这次上屏 `next`。
pub fn record(prev: &str, next: &str) {
    if !is_word(prev) || !is_word(next) {
        return;
    }
    with_table(|t| add(t, prev, next));
    DIRTY.store(true, Ordering::Release);
    maybe_flush();
}

/// 全是汉字（不限长度）。`suggest` 用它代替 `is_word`：联想是只读的，
/// `prev` 长一点（整句上屏）反而更该靠后缀回退救回来。
fn all_han(s: &str) -> bool {
    !s.is_empty() && s.chars().all(is_han)
}

/// 先精确查 `key`，查不到就**逐个丢掉最左边的字**再查（后缀回退）。
///
/// `今天北京` 查不到，但 `北京` 查得到，而「北京后面接什么」和「今天北京后面接
/// 什么」基本是同一件事。取**最长的已知后缀**——越长的后缀越接近整词，别一上来
/// 就退到最短的那个。
///
/// 后缀最少 2 个字：`计算机` 退到单字「机」得到的是一堆跟整词无关的碎片。
/// 单字头词只在 `key` 本身就是一个字时才用（用户真的上屏了「我」，不是从长词里截的）。
///
/// 用户表和内置表**共用这一份规则**：写两遍迟早会一边加了下限、另一边没加，
/// 表现是「桌面能联想、手机不行」这种最难查的分歧。
fn find_with<T>(key: &str, mut get: impl FnMut(&str) -> Option<T>) -> Option<T> {
    if let Some(v) = get(key) {
        return Some(v);
    }
    if key.chars().count() <= 2 {
        return None;
    }
    for (i, _) in key.char_indices().skip(1) {
        let suffix = &key[i..];
        if suffix.chars().count() < 2 {
            break;
        }
        if let Some(v) = get(suffix) {
            return Some(v);
        }
    }
    None
}

/// 合并两张表的后续：用户表整段排前面，内置表只补位，按词去重。
///
/// 权重**不能**跨表比数值：内置是词库统计（几十万量级），用户是选择次数（个位数），
/// 混排会让内置表压死用户刚学的那一条。用户打过的词对他自己永远更可信。
fn merge<'a, 'b, A, B>(mine: Option<A>, theirs: Option<B>, limit: usize) -> Vec<String>
where
    A: Iterator<Item = &'a str>,
    B: Iterator<Item = &'b str>,
{
    let mut out: Vec<String> = Vec::with_capacity(limit);
    push_uniq(&mut out, mine, limit);
    push_uniq(&mut out, theirs, limit);
    out
}

fn push_uniq(out: &mut Vec<String>, it: Option<impl Iterator<Item = impl AsRef<str>>>, limit: usize) {
    let Some(it) = it else { return };
    for w in it {
        if out.len() >= limit {
            return;
        }
        let w = w.as_ref();
        if !out.iter().any(|x| x == w) {
            out.push(w.to_string());
        }
    }
}


/// 查 `prev` 后面的联想词，按「用户表优先、内置表补位」合并，去重。
///
/// 两张表各自做后缀回退（[`resolve`]）：同一个 `prev` 在用户表和内置表里可能落在
/// 不同长度的后缀上，各查各的再合并，比强行统一到同一个键更不容易漏。
///
/// 接受 1..=任意长度的纯汉字 `prev`——单字、三字词、整句上屏都走这一条路。
pub fn suggest(prev: &str, limit: usize) -> Vec<String> {
    if limit == 0 || !all_han(prev) {
        return Vec::new();
    }
    let mine = with_table(|t| find_with(prev, |k| t.get(k)).cloned());
    let theirs = find_with(prev, |k| builtin().find(k));
    merge(
        mine.as_ref().map(|v| v.iter().map(|(w, _)| w.as_str())),
        theirs,
        limit,
    )
}

/// 用户表里有多少个词对（设置窗显示用）。内置表不算——那是发货就带的，不是
/// 「学到的」。
pub fn count() -> usize {
    with_table(|t| t.values().map(|v| v.len()).sum())
}

/// 内置表有多少条词对（设置窗显示用）。
pub fn builtin_count() -> usize {
    builtin().pair_count()
}

/// 清空**用户表**（设置窗的「清空」按钮）。
///
/// 内置表不受影响——它是编译进 DLL 的，用户删不掉也不想删掉；清空之后联想
/// 还在，只是回到「只有内置表」的冷启动状态。
pub fn clear() {
    with_table(|t| t.clear());
    DIRTY.store(true, Ordering::Release);
    flush();
}

/// 重新读盘。用户在别处改了 `next.txt`、或换了配置目录时用。
///
/// 按 mtime 判断，没变就只做一次 `metadata`——`Activate` 会调它（切窗口很频繁）。
pub fn refresh() {
    if DIRTY.load(Ordering::Acquire) {
        // 本进程有没写完的东西，别覆盖掉内存表
        return;
    }
    let mtime = std::fs::metadata(file()).ok().and_then(|m| m.modified().ok());
    {
        let mut g = LAST_MTIME.lock().unwrap_or_else(|e| e.into_inner());
        if *g == mtime {
            return;
        }
        *g = mtime;
    }
    let mut g = TABLE.write().unwrap_or_else(|e| e.into_inner());
    *g = Some(parse(&std::fs::read_to_string(file()).unwrap_or_default()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> Table {
        Table::new()
    }

    #[test]
    fn only_chinese_words_are_recorded() {
        assert!(is_word("北京"));
        assert!(is_word("我"));
        assert!(!is_word("你好，"), "带标点不是词");
        assert!(!is_word("nihao"));
        assert!(!is_word("2026"));
        assert!(!is_word(""));
        assert!(!is_word("一二三四五六七八九"), "超过 8 字不收");
    }

    #[test]
    fn leading_word_stops_at_punctuation() {
        assert_eq!(leading_word("北京，"), "北京");
        assert_eq!(leading_word("你好"), "你好");
        assert_eq!(leading_word("，"), "");
        assert_eq!(leading_word("abc中文"), "");
        assert_eq!(leading_word("2026年"), "");
    }

    #[test]
    fn same_pair_bumps_count_and_sorts_first() {
        let mut m = t();
        add(&mut m, "北京", "时间");
        add(&mut m, "北京", "大学");
        add(&mut m, "北京", "时间");
        assert_eq!(m["北京"][0], ("时间".to_string(), 2), "次数高的排前面");
        assert_eq!(m["北京"][1], ("大学".to_string(), 1));
    }

    #[test]
    fn tail_is_capped() {
        let mut m = t();
        // 九个都是独立的汉字词：`词0` 这种带数字的会被 `is_word` 挡在门外
        for c in ["甲", "乙", "丙", "丁", "戊", "己", "庚", "辛", "壬"] {
            add(&mut m, "北京", c);
        }
        assert_eq!(m["北京"].len(), MAX_TAIL, "一个头词只留 MAX_TAIL 个后续");
    }

    #[test]
    fn weak_heads_are_evicted() {
        let mut m = t();
        // 直接 `insert` 构造慢表（键里的数字无所谓，不走 `is_word`）
        for i in 0..MAX_HEADS {
            m.insert(format!("头{i}"), vec![(format!("尾{i}"), 1)]);
        }
        // 把其中一个喂强，淘汰后它必须活着
        if let Some(v) = m.get_mut("头0") {
            v[0].1 = 50;
        }
        // 新头词必须是真汉字，才会被 `add` 收下
        add(&mut m, "新头", "新尾");
        assert!(m.len() <= MAX_HEADS, "淘汰后不能还超限：{}", m.len());
        assert!(m.contains_key("头0"), "高频头词不能被淘汰");
    }

    #[test]
    fn roundtrip_through_text() {
        let mut m = t();
        add(&mut m, "北京", "时间");
        add(&mut m, "北京", "时间");
        add(&mut m, "我", "们");
        let text = render(&m);
        assert!(text.starts_with('#'), "文件带说明头");
        let back = parse(&text);
        assert_eq!(back["北京"][0], ("时间".to_string(), 2));
        assert_eq!(back["我"][0], ("们".to_string(), 1));
    }

    /// 坏行不能把整张表带崩：用户在编辑器里改错一格是常事。
    #[test]
    fn parse_skips_broken_lines() {
        let text = "北京\t时间\t3\n只一个字段\n\t空头\t2\n北京\t,\t5\n我\t们\t不是数字\n我\t们\t4\n";
        let m = parse(text);
        assert_eq!(m["北京"], vec![("时间".to_string(), 3)]);
        assert_eq!(m["我"], vec![("们".to_string(), 4)]);
        assert!(!m.contains_key(""));
    }

    /// 后缀回退：整词查不到时，取最长的已知后缀。
    #[test]
    fn resolve_falls_back_to_the_longest_suffix() {
        let mut m = t();
        m.insert("北京".to_string(), vec![("时间".to_string(), 5)]);
        m.insert("京".to_string(), vec![("剧".to_string(), 9)]);
        let get = |k: &str| m.get(k);
        assert_eq!(find_with("北京", get).unwrap()[0].0, "时间", "精确命中优先");
        assert_eq!(find_with("今天北京", get).unwrap()[0].0, "时间", "长词靠后缀回退");
        assert_eq!(find_with("去了北京", get).unwrap()[0].0, "时间");
        assert!(find_with("上海", get).is_none(), "没有后缀可用");
        assert!(find_with("南京", get).is_none(), "单字后缀不当退路");
    }

    /// 越长的后缀越接近整词，不能一上来就退到最短的那个。
    #[test]
    fn resolve_prefers_the_longer_suffix() {
        let mut m = t();
        m.insert("京".to_string(), vec![("剧".to_string(), 1)]);
        m.insert("北京".to_string(), vec![("时间".to_string(), 1)]);
        m.insert("天北京".to_string(), vec![("欢迎".to_string(), 1)]);
        let get = |k: &str| m.get(k);
        assert_eq!(find_with("今天北京", get).unwrap()[0].0, "欢迎", "最长后缀优先");
        assert_eq!(find_with("北京", get).unwrap()[0].0, "时间");
    }

    /// 单字头词只在 prev 本身就是一个字时用，不能从长词末尾截一个字凑。
    #[test]
    fn single_char_head_only_matches_a_single_char_prev() {
        let mut m = t();
        m.insert("我".to_string(), vec![("觉得".to_string(), 9)]);
        let get = |k: &str| m.get(k);
        assert_eq!(find_with("我", get).unwrap()[0].0, "觉得");
        assert!(find_with("你们我", get).is_none(), "单字后缀不当退路");
    }

    /// 用户表整段排前面 + 按词去重 + 到 limit 就停。
    #[test]
    fn merge_puts_user_table_first_and_dedups() {
        let mine = ["时间"];
        let theirs = ["时间", "大学"];
        assert_eq!(
            merge(Some(mine.into_iter()), Some(theirs.into_iter()), 5),
            vec!["时间", "大学"]
        );
        assert_eq!(
            merge(None::<std::iter::Empty<&str>>, Some(theirs.into_iter()), 1),
            vec!["时间"],
            "limit 要截断"
        );
        assert!(merge(None::<std::iter::Empty<&str>>, None::<std::iter::Empty<&str>>, 5).is_empty());
    }

    /// suggest 接受任意长度的纯汉字（长词靠后缀回退），但不是纯汉字的直接拒。
    #[test]
    fn suggest_accepts_any_length_of_han() {
        assert!(all_han("我"));
        assert!(all_han("一二三四五六七八九"), "超过 8 字也要接受，后缀回退要用");
        assert!(!all_han("北京1"));
        assert!(!all_han(""));
    }

    /// 内置表必须各长度都有货——这抓的是「只有两字词有联想」那次退化：
    /// 生成器曾经按总权重取前 5000，2 字头词把单字 / 三字头词全挤掉了。
    #[test]
    fn builtin_covers_single_and_long_heads() {
        let t = builtin();
        let mut by_len: std::collections::BTreeMap<usize, usize> = std::collections::BTreeMap::new();
        for (head, _) in t.iter() {
            *by_len.entry(head.chars().count()).or_default() += 1;
        }
        assert!(*by_len.get(&1).unwrap_or(&0) >= 800, "单字头词太少：{by_len:?}");
        assert!(*by_len.get(&2).unwrap_or(&0) >= 4500, "两字头词太少：{by_len:?}");
        assert!(*by_len.get(&3).unwrap_or(&0) >= 500, "三字头词太少：{by_len:?}");
        assert!(*by_len.get(&4).unwrap_or(&0) >= 200, "四字头词太少：{by_len:?}");
        for head in ["我", "不", "计算机", "上市公司"] {
            assert!(t.find(head).is_some(), "内置表缺「{head}」——重新跑 tools/gen_next.py");
        }
    }

    /// 内置表是 build.rs 编进 DLL 的二进制，**它坏了就等于联想功能带病发货**。
    /// 这条测试锁住三件事：规模没被生成参数意外缩小、高频词都在、每一条都干净。
    #[test]
    fn builtin_table_is_intact() {
        let t = builtin();
        assert!(
            t.head_count() >= 6000,
            "内置表头词只有 {} 个，生成参数是不是变了",
            t.head_count()
        );
        assert!(t.pair_count() >= 15000, "内置表词对只有 {} 条", t.pair_count());
        for head in ["北京", "我们", "经济", "工作"] {
            let tails: Vec<&str> = match t.find(head) {
                Some(x) => x.collect(),
                None => panic!("内置表缺「{head}」——重新跑 tools/gen_next.py 看看"),
            };
            assert!(!tails.is_empty() && tails.len() <= MAX_TAIL, "「{head}」的后续数量异常");
            for w in tails {
                assert!(is_word(w), "内置表「{head}」的后续「{w}」不是合法词");
            }
        }
        for (head, tails) in t.iter() {
            assert!(is_word(head), "内置表头词「{head}」不是合法词");
            assert!(tails.len() <= MAX_TAIL, "「{head}」挂了 {} 个后续", tails.len());
        }
    }

    /// 二进制里**不存权重**，运行时按文件顺序取——所以顺序必须就是生成时的词频顺序。
    ///
    /// 这条抓的是一次真实事故：解析时曾经把内置权重 min(MAX_COUNT)（=500）压平，
    /// 「三国」后面的 群英传 / 演义 / 时期 / 时代 全变成 500，排序退化成按字符串比，
    /// 屏幕上先出「时代」。现在没有权重可压平了，但顺序同样可能被生成器或编码器
    /// 打乱，所以直接钉住第一名。
    #[test]
    fn builtin_tail_order_is_preserved() {
        let t = builtin();
        let san: Vec<&str> = t.find("三国").expect("内置表里没有「三国」").collect();
        assert_eq!(san.first().copied(), Some("群英传"), "{san:?}");
        let ban: Vec<&str> = t.find("版权").expect("内置表里没有「版权」").collect();
        assert_eq!(ban.first().copied(), Some("所有"), "{ban:?}");
    }
}
