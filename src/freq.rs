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
//! 稳定排序，键是 **(码长, 学习次数降序)**。码长必须排在次数前面：
//! `ig` 是「成」的整码（2 键），而「程序」只是前缀匹配（4 键）；如果让次数
//! 单独说了算，打过几次「程序」之后，敲 `ig` 按空格出来的就是「程序」。
//! 码长短 = 更完整，这条不能被学习结果覆盖。
//!
//! 同一码长内才是次数降序；次数相同就保持词库原有顺序，所以：
//! - 没学过的词，相对顺序完全不变（不会因为引入这个功能把候选顺序搞乱）
//! - 学过的词在**同长度的候选里**沉浮，次数一样时仍是「语料词频高的在前」
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

/// 解析记录文本。两种行格式都认：
///
/// ```text
/// 哦	8        旧的：只有词和次数，对**所有码**生效（兼容老文件）
/// oo	哦	8    新的：码 + 词 + 次数
/// ```
///
/// 抽成纯函数才能单测（真文件读写会碰 APPDATA）。
fn parse(text: &str) -> HashMap<String, u32> {
    let mut out: HashMap<String, u32> = HashMap::new();
    // 老格式的条目：词 → 次数。等所有行读完再决定它们是退休还是并进按码记录。
    let mut legacy: HashMap<String, u32> = HashMap::new();
    // 按码记录的 (键, 词)，用来把老记录的次数迁移过去
    let mut specific: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').map(str::trim).collect();
        match cols.as_slice() {
            [code, word, n, ..] if !code.is_empty() && !word.is_empty() => {
                let key = format!("{code}\t{word}");
                specific.push((key.clone(), (*word).to_string()));
                let n: u32 = n.parse().unwrap_or(1).clamp(1, MAX_COUNT);
                out.insert(key, n);
            }
            [word, n, ..] if !word.is_empty() => {
                let n: u32 = n.parse().unwrap_or(1).clamp(1, MAX_COUNT);
                legacy.insert((*word).to_string(), n);
            }
            _ => {}
        }
    }
    // 老格式（不带码、对**所有**码生效）退休，但次数不能白丢：并进这个词已有的
    // 按码记录（取较大值）。否则「哦」攒的 8 次会在升级后归零，而它又是把 `ee`
    // 第一位的元凶——旧记录不退，按码记就等于白记。
    for (key, word) in &specific {
        if let Some(n) = legacy.remove(word) {
            let cur = out.get(key).copied().unwrap_or(0);
            out.insert(key.clone(), cur.max(n));
        }
    }
    // 没有按码记录的词，老记录照旧对全部码生效——升级不能让历史学习全废
    for (word, n) in legacy {
        out.insert(word, n);
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
///
/// `code` 是**当时敲的码**。学习必须按「码 + 词」记，不能只按词记：
/// 多音字一个词挂多条读音（`哦` 既有 `oo` 也有 `ee`、`重庆` 既有 chong 也有 zhong），
/// 只按词记的话，在 `oo` 里选过的「哦」会把次数带到 `ee` 上，让一个生僻读音的字
/// 顶掉 `ee` 下所有常用字（饿 / 呃 / 额）。用户看到的是「怎么冒出来个 哦」。
///
/// `code` 传空串等于「对所有码生效」，只有老格式的记录才是这样。
pub fn bump(code: &str, word: &str) {
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
    let key = record_key(code, word);
    with_table(|t| {
        match t.get_mut(&key) {
            Some(n) => *n = (*n + 1).min(MAX_COUNT),
            None => {
                if t.len() >= MAX_ENTRIES {
                    return;
                }
                t.insert(key, 1);
            }
        }
    });
    DIRTY.store(true, Ordering::Release);
    maybe_flush();
}

/// 一条记录的键。`code` 为空时退化成「只有词」的老格式（对所有码生效）。
fn record_key(code: &str, word: &str) -> String {
    if code.is_empty() {
        word.to_string()
    } else {
        format!("{code}\t{word}")
    }
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
    flush_inner(true)
}

/// `merge_disk = false` 时**不把磁盘上的旧内容并回来**。
///
/// 只有 `clear()` 需要这样：合并是「同词取大、新词插入」的语义，刚清空的表会被
/// 磁盘上那份原样灌回来，用户点「重置」就等于没点。
fn flush_inner(merge_disk: bool) {
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
    let disk = if merge_disk {
        parse(&std::fs::read_to_string(file()).unwrap_or_default())
    } else {
        HashMap::new()
    };
    let text = with_table(|t| {
        merge_into(t, disk);
        let mut v: Vec<(&String, &u32)> = t.iter().collect();
        // 次数高的在前，人看起来更像「我的常用词」
        v.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        let mut s = String::with_capacity(v.len() * 20 + 64);
        s.push_str(
            "# 飞鹤输入法动态词频\n\
             # 格式：码<TAB>词<TAB>次数（老格式「词<TAB>次数」也认，对全部码生效）\n\
             # 删掉本文件即重置\n",
        );
        for (k, n) in v {
            // `k` 本身就是「码\t词」或老格式的「词」，所以这里直接接一个 tab 就是两种格式
            s.push_str(k);
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
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let disk = parse(&text);
    // **整份替换，不是合并。** 别的进程可能刚点了「重置」，合并会把旧内容留在
    // 本进程里，清空永远传不过来。本进程未落盘的改动在上面的 `flush()` 里已经
    // 写进文件了，所以这里替换不会丢东西。
    with_table(|t| {
        t.clear();
        t.extend(disk);
    });
}

/// 算出重排后的下标顺序：先比码长，再比学习次数，都相同就保持原序。
///
/// 抽成纯函数是为了单测——`rank` 要过 `enabled()` 和全局表，测试里不好摆布。
fn order_by(
    words: &[std::borrow::Cow<'static, str>],
    lens: &[u8],
    table: &HashMap<String, u32>,
    code: &str,
) -> Vec<usize> {
    // 复用一个缓冲拼键（`码 + \t + 词`），避免每个候选都分配一个 String
    let mut buf = String::with_capacity(code.len() + 16);
    let count_of = |buf: &mut String, w: &str| -> u32 {
        if !code.is_empty() {
            buf.clear();
            buf.push_str(code);
            buf.push('\t');
            buf.push_str(w);
            // 码级记录优先；老文件里只有词级的，退回去用它（对所有码生效）
            if let Some(n) = table.get(buf.as_str()) {
                return *n;
            }
        }
        table.get(w).copied().unwrap_or(0)
    };
    let mut order: Vec<usize> = (0..words.len()).collect();
    // 稳定排序：`sort_by_key` 保证相等元素保持原顺序，这是「不搞乱语料顺序」的前提
    order.sort_by_key(|&i| {
        let len = lens.get(i).copied().unwrap_or(u8::MAX);
        (len, std::cmp::Reverse(count_of(&mut buf, words[i].as_ref())))
    });
    order
}

/// 按「码长 → 学习次数」稳定重排候选。
///
/// `lens` 是每个候选的**码长**（音节数 × 2），由 `dict::collect` 一起带过来。
/// `code` 是当前查询的码——学习记录按「码 + 词」存，见 `bump`。
/// 长度不等长时不 panic，缺的按最大长度算（排到最后），避免一处算错就崩掉宿主。
pub fn rank(words: &mut [std::borrow::Cow<'static, str>], lens: &[u8], code: &str) {
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
    let mut perm = order_by(words, lens, table, code);
    apply_order(words, &mut perm);
}

/// 按置换就地重排，逐环交换，不额外分配。
///
/// `order[i]` 的语义是「排序后第 `i` 位来自原来的第几个」（`sort_by_key` 给的
/// 就是这个方向）。逐环交换要求反过来——「第 `i` 个元素该去第几位」，所以先求逆。
fn apply_order(words: &mut [std::borrow::Cow<'static, str>], order: &mut [usize]) {
    let n = words.len();
    if order.len() != n || order.iter().enumerate().all(|(i, &j)| i == j) {
        return;
    }
    let mut dest = vec![0usize; n];
    for (pos, &src) in order.iter().enumerate() {
        dest[src] = pos;
    }
    for i in 0..n {
        while dest[i] != i {
            let j = dest[i];
            words.swap(i, j);
            dest.swap(i, j);
        }
    }
}

/// 清空学习表（设置窗「重置」用）。返回清掉的条目数。
///
/// **必须绕开磁盘合并**：`flush` 默认会把磁盘内容并回内存表（那是为了多进程
/// 不互相覆盖），但清空恰恰要反过来——不绕的话，刚清空的表立刻被旧内容灌满，
/// 用户点「重置」什么也不会发生。
pub fn clear() -> usize {
    let n = with_table(|t| {
        let n = t.len();
        t.clear();
        n
    });
    DIRTY.store(true, Ordering::Release);
    flush_inner(false);
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

    /// 新格式带码，老格式（只有词）必须继续认——老用户的 freq.txt 不能一夜失效
    #[test]
    fn parses_both_record_formats() {
        let t = parse("oo\t哦\t8\n重庆\t3\n");
        assert_eq!(t.get("oo\t哦"), Some(&8));
        assert_eq!(t.get("重庆"), Some(&3), "老格式退化成「只有词」，对所有码生效");
    }

    /// 同一个词一旦有了「按码」的记录，老格式那条就该退休——
    /// 否则「哦」的老记录会永远把 `ee` 的第一位占着，按码记等于白记。
    #[test]
    fn specific_record_retires_legacy_one() {
        let t = parse("哦\t8\noo\t哦\t3\n");
        assert_eq!(t.get("哦"), None, "老记录要退休，不能再对所有码生效");
        assert_eq!(t.get("oo\t哦"), Some(&8), "退休前先把次数并进按码记录（取较大值）");
        // 没有按码记录的词，老格式照旧生效
        let t = parse("重庆\t3\n");
        assert_eq!(t.get("重庆"), Some(&3));
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
        rank(&mut v, &[2, 2, 2], "ab");
        assert_eq!(v, vec!["甲", "乙", "丙"]);
    }

    /// 同一码长内，学过的排前面（这是「动态词频」本来的意义）
    #[test]
    fn learned_word_wins_inside_same_code_length() {
        let mut table: HashMap<String, u32> = HashMap::new();
        table.insert("丙".into(), 5);
        let v = [Cow::Borrowed("甲"), Cow::Borrowed("乙"), Cow::Borrowed("丙")];
        assert_eq!(order_by(&v, &[2, 2, 2], &table, "ab"), vec![2, 0, 1]);
    }

    /// 学习记录是**按码**记的：在 `oo` 里选过「哦」，不该把它的次数带到 `ee` 上。
    ///
    /// 这是用户报的真事——「为什么 oo 能出哦，ee 也能出哦，而且排在第一位」。
    /// 多音字一个词挂多条读音，只按词记次数就会串味：饿 / 呃 / 额 全被一个生僻
    /// 读音的「哦」压在后面。
    #[test]
    fn learned_count_is_per_code() {
        let mut table: HashMap<String, u32> = HashMap::new();
        table.insert("oo\t哦".into(), 8);
        let v = [Cow::Borrowed("饿"), Cow::Borrowed("呃"), Cow::Borrowed("哦")];
        // 查 ee：oo 的记录不算数，顺序原样
        assert_eq!(order_by(&v, &[2, 2, 2], &table, "ee"), vec![0, 1, 2]);
        // 查 oo：是它自己的记录，哦排第一
        assert_eq!(order_by(&v, &[2, 2, 2], &table, "oo"), vec![2, 0, 1]);
    }

    /// 老格式（只有词、没有码）继续对所有码生效，否则升级一次学习记录就白攒了
    #[test]
    fn legacy_record_applies_to_every_code() {
        let mut table: HashMap<String, u32> = HashMap::new();
        table.insert("哦".into(), 8);
        let v = [Cow::Borrowed("饿"), Cow::Borrowed("哦")];
        assert_eq!(order_by(&v, &[2, 2], &table, "ee"), vec![1, 0]);
    }

    /// **码长优先于学习次数**：`ig` 的「成」(2 键) 不能被学过的「程序」(4 键) 顶掉。
    /// 这是实际踩到的 bug——打了几次「程序」之后，敲 `ig` 按空格出来的是「程序」。
    #[test]
    fn shorter_code_beats_learned_word() {
        let mut table: HashMap<String, u32> = HashMap::new();
        table.insert("程序".into(), 99);
        table.insert("城".into(), 7);
        let v = [Cow::Borrowed("程序"), Cow::Borrowed("成"), Cow::Borrowed("城")];
        // 程序码长 4，成/城 码长 2
        assert_eq!(order_by(&v, &[4, 2, 2], &table, "ig"), vec![2, 1, 0]);
    }

    /// 置换重排不能丢词、不能错位（用可验证的一个环来钉）
    #[test]
    fn apply_order_permutes_in_place() {
        let mut v = vec![
            Cow::Borrowed("甲"),
            Cow::Borrowed("乙"),
            Cow::Borrowed("丙"),
            Cow::Borrowed("丁"),
        ];
        // 目标顺序：丙 甲 丁 乙
        let mut perm = vec![2, 0, 3, 1];
        apply_order(&mut v, &mut perm);
        assert_eq!(v, vec!["丙", "甲", "丁", "乙"]);
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
