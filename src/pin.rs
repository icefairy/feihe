//! 手动置顶：把一个词钉死到某个编码上，以后这个码的第一个候选永远是它。
//!
//! ## 为什么需要它
//!
//! 一级简码解决的是「`d` 该出什么」这种**通用**问题；置顶解决的是「`iuliqi`
//! 该出什么」这种**个人**问题。动态词频也能把词往上顶，但它有两个够不着的地方：
//!
//! 1. 只能把**已经在候选里**的词往前挪（排序发生在 `limit` 截断之后），排到
//!    第 90 名以外的词学多少次都捞不上来
//! 2. 是「慢慢学」——同一个词得选上好几次才见效
//!
//! 置顶是一次性的、确定的：用户说了算，立刻生效，而且不随使用量漂移。
//!
//! ## 优先级
//!
//! `手动置顶 > 一级简码 > 动态词频 > 静态词频`
//!
//! 置顶压在一简字上面，是因为一简字只是「默认值」，用户显式钉过的东西
//! 必须赢——否则「我明明置顶了，怎么还是出『的』」会变成一个新 bug。
//!
//! ## 文件
//!
//! `%APPDATA%\Feihe\pinned.txt`，每行 `编码<TAB>词`：
//!
//! ```text
//! # 飞鹤输入法手动置顶
//! d<TAB>大
//! iuliqi<TAB>处理器
//! ```
//!
//! 和 `freq.txt` / `user_dict.txt` 一个风格：能看、能手改、能删。删掉整个文件
//! 就是「全部取消置顶」。编码是**查词用的那个串**（双拼是双拼码，全拼模式是
//! 全拼串），所以同一组键在两种模式下的置顶是各自独立的。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

/// 编码 → 置顶词。`Arc<str>` 是为了让 `word_for` 的克隆只加一次引用计数，
/// 不至于每次按键都为这个词分配一次内存。
type Table = HashMap<String, Arc<str>>;

static TABLE: RwLock<Option<Table>> = RwLock::new(None);
/// 上次读盘时文件的修改时间，用来判断别的进程有没有改过
static LAST_MTIME: Mutex<Option<std::time::SystemTime>> = Mutex::new(None);

pub fn file() -> std::path::PathBuf {
    crate::config::dir().join("pinned.txt")
}

/// 解析 `编码<TAB>词` 文本。抽成纯函数才能单测（真文件读写会碰 APPDATA）。
///
/// 同一个编码出现多次时**后面的赢**——手改文件的人习惯把新想法写在下面。
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
        out.insert(code.to_ascii_lowercase(), Arc::from(word));
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
        // 锁中毒（某个线程 panic 了）：当成空表继续跑，绝不能因此崩掉宿主
        Err(e) => e.into_inner(),
    };
    f(g.get_or_insert_with(HashMap::new))
}

/// 这个编码被置顶的词。没有就是 `None`。
pub fn word_for(code: &str) -> Option<Arc<str>> {
    if code.is_empty() {
        return None;
    }
    ensure_loaded();
    let g = TABLE.read().ok()?;
    g.as_ref()?.get(code).cloned()
}

/// 置顶：`code` 这个编码以后第一个候选固定是 `word`。已有则覆盖。
pub fn set(code: &str, word: &str) {
    if code.is_empty() || word.is_empty() {
        return;
    }
    let code = code.to_ascii_lowercase();
    mutate(|t| {
        t.insert(code, Arc::from(word));
    });
}

/// 取消某个编码的置顶。返回原来有没有。
pub fn remove(code: &str) -> bool {
    let code = code.to_ascii_lowercase();
    mutate(|t| t.remove(&code).is_some())
}

/// 切换：这个码上已经是 `word` 就取消，否则钉上。返回切换后是否处于置顶状态。
///
/// 「置顶」和「取消置顶」是同一个动作的两面（`Ctrl`+数字 和右键点候选都要它），
/// 用一个入口切换，用户不必再记第二个组合。
pub fn toggle(code: &str, word: &str) -> bool {
    let next = decide(word_for(code).as_deref(), word);
    match next {
        Toggle::Clear => {
            remove(code);
            false
        }
        Toggle::Set => {
            set(code, word);
            true
        }
    }
}

/// 切换的下一步动作。抽成纯函数：`toggle` 要落盘（碰 `%APPDATA%`），
/// 判据本身才是容易写错的地方（「同一个词才算取消」）。
#[derive(Debug, PartialEq, Eq)]
enum Toggle {
    Set,
    Clear,
}

fn decide(cur: Option<&str>, word: &str) -> Toggle {
    if word.is_empty() {
        return Toggle::Clear;
    }
    match cur {
        Some(c) if c == word => Toggle::Clear,
        _ => Toggle::Set,
    }
}

/// 全部置顶，按编码排序（设置窗显示、调试用）。
pub fn list() -> Vec<(String, String)> {
    with_table(|t| {
        let mut v: Vec<(String, String)> = t
            .iter()
            .map(|(c, w)| (c.clone(), w.to_string()))
            .collect();
        v.sort();
        v
    })
}

/// 清空全部置顶，返回清掉的条数。
pub fn clear() -> usize {
    mutate(|t| {
        let n = t.len();
        t.clear();
        n
    })
}

/// 所有写操作都走这里：**先读盘、再改、再写盘**。
///
/// 置顶表很小（几十行），每次整读整写完全不是问题，换来的是「删除」和「清空」
/// 能真正生效。
///
/// 不能学 `freq::flush` 那套「内存改完再合并磁盘」——合并是 `or_insert` 语义，
/// 刚删掉的条目会被磁盘旧内容原样灌回来，表现为「点了清空没反应」。
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
        let mut v: Vec<(&String, &Arc<str>)> = t.iter().collect();
        v.sort_by(|a, b| a.0.cmp(b.0));
        let mut s = String::with_capacity(v.len() * 24 + 64);
        s.push_str("# 飞鹤输入法手动置顶\n# 格式：编码<TAB>词（删掉本文件即全部取消）\n");
        for (c, w) in v {
            s.push_str(c);
            s.push('\t');
            s.push_str(w);
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

/// 别的进程改过 `pinned.txt` 之后把新内容拉进内存。**在 `Activate` 里调**。
///
/// 只在 mtime 变了的时候才真读盘——`Activate` 是高频动作（切窗口、切输入法），
/// 每次都解析一遍文本没必要。
pub fn refresh() {
    let path = file();
    let Some(m) = std::fs::metadata(&path).ok().and_then(|x| x.modified().ok()) else {
        // 文件被删了 → 内存里也不该再留着置顶。
        // 先读锁看一眼，避免每次 Activate 都去抢写锁。
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
    // 整份替换而不是合并：置顶是「以文件为准」的表，删掉一行就该消失
    let disk = load_from_disk();
    with_table(|t| {
        t.clear();
        t.extend(disk);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 同一个词再切一次 = 取消；换一个词 = 覆盖。这是 `Ctrl`+数字 和右键共用的判据。
    #[test]
    fn toggle_decides_by_current_word() {
        assert_eq!(decide(None, "成"), Toggle::Set, "本来没钉 → 钉上");
        assert_eq!(decide(Some("成"), "成"), Toggle::Clear, "同一个词 → 取消");
        assert_eq!(decide(Some("城"), "成"), Toggle::Set, "换了词 → 覆盖");
        assert_eq!(decide(Some("成"), ""), Toggle::Clear, "空词不钉");
    }

    #[test]
    fn parses_pairs_and_ignores_junk() {
        let t = parse("# 注释\nd\t大\niuliqi\t处理器\n坏行没有制表符\n\n d \t 小 \n\t空码\n");
        assert_eq!(t.get("iuliqi").map(|s| &**s), Some("处理器"));
        // 同一个编码写了两遍：手改文件的人习惯把新想法写在下面，后面的赢
        assert_eq!(t.get("d").map(|s| &**s), Some("小"));
        assert_eq!(t.len(), 2, "坏行和空码都要跳过");
    }

    #[test]
    fn code_is_lowercased() {
        // 宿主偶尔会送大写 VK；查的时候统一按小写找
        let t = parse("DE\t的\n");
        assert!(t.contains_key("de"));
    }

    #[test]
    fn empty_query_never_pinned() {
        // 空编码不能匹配到任何置顶，否则一开组合串就冒出个词
        assert!(word_for("").is_none());
    }

    /// 置顶的语义是「这个码第一个出它」，即使它压根不在候选里。
    /// 这条测的是 `dict` 里的提权逻辑，放在这里是因为规则由本模块定义。
    #[test]
    fn promote_moves_to_front_and_inserts_missing() {
        use std::borrow::Cow;
        let mut v: Vec<Cow<'static, str>> = vec!["甲".into(), "乙".into(), "丙".into()];
        super::super::dict::promote_for_test(&mut v, "丙", true, 3);
        assert_eq!(v, vec!["丙", "甲", "乙"], "在列表里就提上来");

        let mut v: Vec<Cow<'static, str>> = vec!["甲".into(), "乙".into()];
        super::super::dict::promote_for_test(&mut v, "丁", true, 2);
        assert_eq!(v, vec!["丁", "甲"], "不在列表里就塞进去并截到 limit");

        let mut v: Vec<Cow<'static, str>> = vec!["甲".into(), "乙".into()];
        super::super::dict::promote_for_test(&mut v, "丁", false, 2);
        assert_eq!(v, vec!["甲", "乙"], "不允许塞入时原样不动");
    }
}
