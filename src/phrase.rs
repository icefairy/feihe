//! 自造词组：记住用户**一段一段拼出来**的词库外新词。
//!
//! ## 为什么需要它
//!
//! 词库再大也有它不认识的东西——人名、公司名、生造的缩写。用户的办法是把一长串
//! 编码切开、一段一段选字，「李明远」就是这么拼出来的（`li`+`mk`+`yr`）。
//! 但这个词只活在**那一次上屏**里：下一次再敲 `limkyr`，词库照样不认识它，
//! 于是又得回头拆一遍词、再选一遍字。用户的原话是「我刚组出来的词怎么又没了」。
//!
//! 主流输入法把这个叫「自动造词 / 智能组词」，干的正是这件事：把用户这次拼出来的
//! 东西记下来，下次直接给。
//!
//! ## 只记「用户真的点过」的
//!
//! 触发点在 `textservice::commit_segments`（分词上屏）。三个条件缺一不可，
//! 都是为了**不记垃圾**：至少两段、每段都真有词、段码拼起来等于用户敲的那串。
//! 另外词库里本来就有的词一律跳过——自动切分沿用默认候选（用户没改）时
//! 不该往表里灌一份重复数据。
//!
//! ## 存全拼，不存码
//!
//! 和 `pin` 不一样：置顶存的是「编码→词」，换方案之后那条置顶就失效（用户自己
//! 钉的，认了）；但自造词是自动记的，用户不知道有这回事，换方案后突然不灵了
//! 只会觉得是 bug。所以这里存**全拼**，查询时按当前方案现场编码——和小鹤/微软
//! 双拼、全拼三种索引都对得上，切方案、开全拼模式照样能打出来。
//!
//! ## 文件
//!
//! `%APPDATA%\Feihe\phrases.txt`，每行 `词<TAB>全拼`，越靠前越新：
//!
//! ```text
//! # 飞鹤输入法自造词组
//! 李明远<TAB>li ming yuan
//! ```
//!
//! 和 `freq.txt` / `pinned.txt` 一个风格：能看、能手改、能删。删掉整个文件
//! 就是「忘掉所有自造词」。

use std::sync::{Mutex, RwLock};

use crate::dictbin::Idx;
use crate::shuangpin::{self, Scheme};

/// 最多记多少条。到上限淘汰最久没学的（列表尾）。自造词是低频事件，
/// 2000 条对一个每天都打字的人也是好几年的量。
const MAX_ENTRIES: usize = 2000;

struct Entry {
    word: Box<str>,
    /// 音节用空格分隔的全拼（`li ming yuan`）
    pinyin: Box<str>,
}

/// 表：条目按「最近学的在前」排，外加三张按码升序的索引（和 `dict::Overlay` 同构）。
struct Table {
    entries: Vec<Entry>,
    xh: Vec<(Box<str>, u32)>,
    ms: Vec<(Box<str>, u32)>,
    flat: Vec<(Box<str>, u32)>,
}

impl Table {
    fn build(entries: Vec<Entry>) -> Self {
        let mut xh = Vec::with_capacity(entries.len());
        let mut ms = Vec::with_capacity(entries.len());
        let mut flat = Vec::with_capacity(entries.len());
        for (i, e) in entries.iter().enumerate() {
            if let Some(c) = shuangpin::encode_pinyin(Scheme::Xiaohe, &e.pinyin) {
                xh.push((c.into_boxed_str(), i as u32));
            }
            if let Some(c) = shuangpin::encode_pinyin(Scheme::Ms, &e.pinyin) {
                ms.push((c.into_boxed_str(), i as u32));
            }
            let f: String = e.pinyin.chars().filter(|c| !c.is_whitespace()).collect();
            if !f.is_empty() {
                flat.push((f.into_boxed_str(), i as u32));
            }
        }
        // 稳定排序：同码内保持 entries 顺序，也就是「新学的在前」
        xh.sort_by(|a, b| a.0.cmp(&b.0));
        ms.sort_by(|a, b| a.0.cmp(&b.0));
        flat.sort_by(|a, b| a.0.cmp(&b.0));
        Self {
            entries,
            xh,
            ms,
            flat,
        }
    }

    fn index(&self, which: Idx) -> &[(Box<str>, u32)] {
        match which {
            Idx::Xiaohe => &self.xh,
            Idx::Ms => &self.ms,
            Idx::Flat => &self.flat,
        }
    }

    /// 这个码（**整码**，不是前缀）下学过的词，新学的在前。
    ///
    /// 只认整码：前缀查询时把半个码的候选混进来，用户看到的是「我还没打完它就出来了」，
    /// 而且会和正常的自动切分打架。
    fn words_with_code(&self, which: Idx, code: &str) -> Vec<String> {
        let arr = self.index(which);
        let start = arr.partition_point(|(c, _)| c.as_ref() < code);
        let mut out = Vec::new();
        for (c, i) in &arr[start..] {
            if c.as_ref() != code {
                break;
            }
            out.push(self.entries[*i as usize].word.to_string());
        }
        out
    }
}

static TABLE: RwLock<Option<Table>> = RwLock::new(None);
/// 上次读盘时文件的修改时间，用来判断别的进程有没有改过
static LAST_MTIME: Mutex<Option<std::time::SystemTime>> = Mutex::new(None);

pub fn file() -> std::path::PathBuf {
    crate::config::dir().join("phrases.txt")
}

/// 解析 `词<TAB>全拼` 文本。抽成纯函数才能单测（真文件读写会碰 APPDATA）。
fn parse(text: &str) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((w, py)) = line.split_once('\t') else {
            continue;
        };
        let (w, py) = (w.trim(), py.trim());
        if w.is_empty() || py.is_empty() {
            continue;
        }
        // 全拼编不出码的行（手改坏了）直接丢：留着也永远查不到，
        // 还会在写回时把文件撑大。
        if shuangpin::encode_pinyin(Scheme::Xiaohe, py).is_none() {
            continue;
        }
        out.push(Entry {
            word: w.into(),
            pinyin: py.into(),
        });
    }
    out.truncate(MAX_ENTRIES);
    out
}

fn load_from_disk() -> Vec<Entry> {
    parse(&std::fs::read_to_string(file()).unwrap_or_default())
}

fn ensure_loaded() {
    if TABLE.read().map(|g| g.is_none()).unwrap_or(false) {
        let t = Table::build(load_from_disk());
        if let Ok(mut g) = TABLE.write() {
            if g.is_none() {
                *g = Some(t);
            }
        }
    }
}

fn with_table<R>(f: impl FnOnce(&mut Table) -> R) -> R {
    ensure_loaded();
    let mut g = match TABLE.write() {
        Ok(g) => g,
        // 锁中毒（某个线程 panic 了）：当成空表继续跑，绝不能因此崩掉宿主
        Err(e) => e.into_inner(),
    };
    f(g.get_or_insert_with(|| Table::build(Vec::new())))
}

/// 这个码下学过的词，新学的在前。`pinyin` 表示当前是全拼模式。
pub fn words_for(code: &str, pinyin: bool) -> Vec<String> {
    if code.is_empty() {
        return Vec::new();
    }
    ensure_loaded();
    let g = match TABLE.read() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    let Some(t) = g.as_ref() else {
        return Vec::new();
    };
    let which = if pinyin {
        Idx::Flat
    } else {
        match crate::dict::scheme() {
            Scheme::Xiaohe => Idx::Xiaohe,
            Scheme::Ms => Idx::Ms,
        }
    };
    t.words_with_code(which, code)
}

/// 记一个新词。同一个词再记一次只是把它挪到最前面（下次优先出它）。
///
/// 写盘策略和 `pin` 一样：**先读盘、再改、再写盘**。自造词是低频事件（一个人
/// 一天也不会造几个），每次整读整写完全不是问题，换来的是多进程之间不互相覆盖。
pub fn learn(word: &str, pinyin: &str) {
    if word.is_empty() || word.is_ascii() || pinyin.is_empty() {
        return;
    }
    if shuangpin::encode_pinyin(Scheme::Xiaohe, pinyin).is_none() {
        return;
    }
    let mut entries = load_from_disk();
    entries.retain(|e| e.word.as_ref() != word);
    entries.insert(
        0,
        Entry {
            word: word.into(),
            pinyin: pinyin.into(),
        },
    );
    entries.truncate(MAX_ENTRIES);

    let table = Table::build(entries);
    if let Ok(mut g) = TABLE.write() {
        *g = Some(table);
    }
    write_now();
}

/// 学到的词条数（设置窗显示用）
pub fn len() -> usize {
    with_table(|t| t.entries.len())
}

/// 清空自造词，返回清掉的条数。删掉整个文件即「忘掉所有自造词」。
pub fn clear() -> usize {
    let n = with_table(|t| {
        let n = t.entries.len();
        *t = Table::build(Vec::new());
        n
    });
    write_now();
    n
}

/// 把内存表整份写盘（原子改名），顺序就是「新学的在前」。
fn write_now() {
    // 测试进程绝不写盘（和 config::save 同样的理由）
    if cfg!(test) {
        return;
    }
    let text = with_table(|t| {
        let mut s = String::with_capacity(t.entries.len() * 24 + 64);
        s.push_str("# 飞鹤输入法自造词组\n# 格式：词<TAB>全拼（删掉本文件即清空）\n");
        for e in &t.entries {
            s.push_str(&e.word);
            s.push('\t');
            s.push_str(&e.pinyin);
            s.push('\n');
        }
        s
    });

    let _ = std::fs::create_dir_all(crate::config::dir());
    let path = file();
    let tmp = path.with_extension("txt.tmp");
    if std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    // 记下自己刚写的 mtime，免得紧接着的 refresh() 白读一遍
    if let Some(m) = std::fs::metadata(&path).ok().and_then(|x| x.modified().ok()) {
        if let Ok(mut g) = LAST_MTIME.lock() {
            *g = Some(m);
        }
    }
}

/// 别的进程记过新词之后把内容拉进来。**在 `Activate` 里调**（切窗口、切输入法时）。
///
/// 只在 mtime 变了的时候才真读盘——`Activate` 是高频动作。
pub fn refresh() {
    let path = file();
    let Some(m) = std::fs::metadata(&path).ok().and_then(|x| x.modified().ok()) else {
        // 文件被删了 → 内存里也不该再留着自造词（用户点「清空」或手删了文件）
        let nonempty = TABLE
            .read()
            .map(|g| g.as_ref().is_some_and(|t| !t.entries.is_empty()))
            .unwrap_or(false);
        if nonempty {
            with_table(|t| *t = Table::build(Vec::new()));
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
    // 整份替换而不是合并：删掉一行就该消失
    let disk = load_from_disk();
    if let Ok(mut g) = TABLE.write() {
        *g = Some(Table::build(disk));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ent(w: &str, py: &str) -> Entry {
        Entry {
            word: w.into(),
            pinyin: py.into(),
        }
    }

    #[test]
    fn parses_pairs_and_ignores_junk() {
        let v = parse("# 注释\n李明远\tli ming yuan\n坏行没有制表符\n\n 甲 \t jia \n\t说明\n");
        assert_eq!(v.len(), 2, "坏行、空词、空拼音都要跳过");
        assert_eq!(v[0].word.as_ref(), "李明远");
        assert_eq!(v[0].pinyin.as_ref(), "li ming yuan");
        assert_eq!(v[1].word.as_ref(), "甲");
    }

    /// 编不出码的全拼（手改坏了）不能留在表里：留着永远查不到，还会一直写回文件
    #[test]
    fn drops_unencodable_pinyin() {
        let v = parse("坏词\tzzz qqq\n好词\tli ming yuan\n");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].word.as_ref(), "好词");
    }

    /// 同一个码学了两个词：都留着，新学的排前面。
    /// 这条是「刚组的词必须出得来」的核心——排在后面就等于没学会。
    #[test]
    fn newest_first_within_same_code() {
        let t = Table::build(vec![
            ent("李明源", "li ming yuan"),
            ent("李明远", "li ming yuan"),
        ]);
        let got = t.words_with_code(Idx::Xiaohe, "limkyr");
        assert_eq!(got, vec!["李明源", "李明远"], "entries 顺序即新旧顺序");
    }

    /// 三套码都要能对上：小鹤、微软、全拼去空格。
    #[test]
    fn all_three_indexes_match() {
        let t = Table::build(vec![ent("李明远", "li ming yuan")]);
        assert_eq!(t.words_with_code(Idx::Xiaohe, "limkyr").len(), 1);
        assert_eq!(t.words_with_code(Idx::Flat, "limingyuan").len(), 1);
        // 微软双拼：li=l+i、ming=m+`;`、yuan=y+r
        assert_eq!(t.words_with_code(Idx::Ms, "lim;yr").len(), 1);
        // 别的码不能命中（整码匹配，不做前缀）
        assert!(t.words_with_code(Idx::Xiaohe, "limk").is_empty());
        assert!(t.words_with_code(Idx::Xiaohe, "limkyrr").is_empty());
    }

    /// 学一个词之后必须立刻查得到（写盘在测试里被跳过，内存表要先生效）。
    /// 查全拼索引，这样和当前双拼方案无关，测试之间不会互相干扰。
    #[test]
    fn learn_writes_through_to_query() {
        learn("李明远", "li ming yuan");
        assert!(
            words_for("limingyuan", true).iter().any(|w| w == "李明远"),
            "学完立刻要能查到"
        );
        // 同一个词再学一次不该产生重复条目
        learn("李明远", "li ming yuan");
        assert_eq!(
            words_for("limingyuan", true)
                .iter()
                .filter(|w| *w == "李明远")
                .count(),
            1
        );
    }
}
