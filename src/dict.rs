//! 词库：**静态二进制 + 运行时用户词库（两种形态）**
//!
//! ## 静态部分
//!
//! `src/dict.txt`（+ 开发机的 `src/user_dict.txt`）在构建期被 `build.rs` 编码成
//! `base.bin`：三套码（小鹤 / 微软 / 全拼去空格）各一张按码升序的下标表。运行时通过
//! `include_bytes!` 嵌在 DLL 的 `.rdata` 段里——那一段由 PE 加载器 mmap，
//! **多个进程共享同一份物理页**，所以词库的常驻内存不再是「每进程 9.2MB 私有堆」。
//!
//! 好处是切双拼方案不再需要重建索引（只是换一张表），`Activate` 里那 81ms 归零。
//!
//! ## 用户词库：txt 是源，bin 是产物
//!
//! `%APPDATA%\Feihe\user_dict.txt` 是人可读、可手改、可导出的**源文件**；
//! `%APPDATA%\Feihe\user.bin` 是它编译出来的 FHDB 二进制（`dictenc` 同一个编码器）。
//!
//! 运行时优先用 `user.bin`：`dictbin::map_file` 把它映射进地址空间，多进程共享
//! 页缓存，查询走和 `base.bin` 完全相同的零解析二分。**不会退化成每进程建一份堆索引。**
//!
//! 只有在 `user.bin` 不存在、或比 `user_dict.txt` 旧（说明用户手改了 txt 还没重新编译）
//! 时，才退回「解析 txt 到内存」的覆盖层。那条路要 125ms/5 万条、每进程一份私有堆，
//! 是过渡形态，不是常态。
//!
//! 导入词库、改完 txt 之后都应该重新编译一次（设置窗里有按钮，导入时会自动做）。

use std::borrow::Cow;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, RwLock};

use crate::assist::{self, AssistMode};
use crate::config;
use crate::dictbin::{self, Bin, Idx};
use crate::dictenc;
use crate::shuangpin::{self, Scheme};

/// 当前双拼方案。缓存在原子变量里，避免每次按键都去抢 `config` 的锁。
static SCHEME: AtomicU8 = AtomicU8::new(0);

pub fn set_scheme(s: Scheme) {
    SCHEME.store(
        match s {
            Scheme::Xiaohe => 0,
            Scheme::Ms => 1,
        },
        Ordering::Release,
    );
}

pub fn scheme() -> Scheme {
    if SCHEME.load(Ordering::Acquire) == 1 {
        Scheme::Ms
    } else {
        Scheme::Xiaohe
    }
}

// ─────────────────────────────────────────────────────────────
// 覆盖层
// ─────────────────────────────────────────────────────────────

struct Overlay {
    /// (词, 全拼)
    entries: Vec<(Box<str>, Box<str>)>,
    /// 三张按码升序的表：(码, entries 下标)
    xh: Vec<(Box<str>, u32)>,
    ms: Vec<(Box<str>, u32)>,
    flat: Vec<(Box<str>, u32)>,
}

impl Overlay {
    fn load() -> Self {
        let text = std::fs::read_to_string(user_file()).unwrap_or_default();
        Self::parse(&text)
    }

    fn parse(text: &str) -> Self {
        let mut entries: Vec<(Box<str>, Box<str>)> = Vec::new();
        let mut seen: HashSet<&str> = HashSet::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((w, py)) = line.split_once('\t') else {
                continue;
            };
            let (w, py) = (w.trim(), py.trim());
            if w.is_empty() || py.is_empty() || !seen.insert(w) {
                continue;
            }
            entries.push((w.into(), py.into()));
        }

        let mut xh = Vec::new();
        let mut ms = Vec::new();
        let mut flat = Vec::new();
        for (i, (_, py)) in entries.iter().enumerate() {
            if let Some(c) = shuangpin::encode_pinyin(Scheme::Xiaohe, py) {
                xh.push((c.into_boxed_str(), i as u32));
            }
            if let Some(c) = shuangpin::encode_pinyin(Scheme::Ms, py) {
                ms.push((c.into_boxed_str(), i as u32));
            }
            let f: String = py.chars().filter(|c| !c.is_whitespace()).collect();
            if !f.is_empty() {
                flat.push((f.into_boxed_str(), i as u32));
            }
        }
        // 稳定排序 → 同码内保持文件顺序
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

    fn table(&self, which: Idx) -> &[(Box<str>, u32)] {
        match which {
            Idx::Xiaohe => &self.xh,
            Idx::Ms => &self.ms,
            Idx::Flat => &self.flat,
        }
    }

    /// 前缀区间（同 `Bin::prefix_range`）
    fn prefix_range(&self, which: Idx, prefix: &str) -> (usize, usize) {
        let arr = self.table(which);
        let s = arr.partition_point(|(c, _)| c.as_ref() < prefix);
        let mut e = s;
        while e < arr.len() && arr[e].0.starts_with(prefix) {
            e += 1;
        }
        (s, e)
    }

    /// 整码区间的上界（同 `Bin::exact_end`）
    fn exact_end(&self, which: Idx, code: &str) -> usize {
        let arr = self.table(which);
        arr.partition_point(|(c, _)| c.as_ref() <= code)
    }

    fn word_at(&self, which: Idx, p: usize) -> &str {
        self.entries[self.table(which)[p].1 as usize].0.as_ref()
    }

    #[allow(dead_code)]
    fn prefix<'a>(&'a self, which: Idx, prefix: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        let (s, e) = self.prefix_range(which, prefix);
        (s..e).map(move |p| self.word_at(which, p))
    }
}

// ─────────────────────────────────────────────────────────────
// 用户词库：user.bin（mmap，常态） / user_dict.txt（回退）
// ─────────────────────────────────────────────────────────────

/// `%APPDATA%\Feihe\user.bin` —— `user_dict.txt` 编译出来的 FHDB
pub fn user_bin_file() -> PathBuf {
    config::dir().join("user.bin")
}

struct BinCache {
    /// 是否已经探测过（`false` 时 `bin` 无意义）
    probed: bool,
    bin: Option<&'static Bin>,
}

static USER_BIN: RwLock<BinCache> = RwLock::new(BinCache {
    probed: false,
    bin: None,
});

fn mtime(p: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).ok()?.modified().ok()
}

/// 探测并映射 `user.bin`。不该用的时候返回 `None`。
///
/// 判据是「产物不能比源旧」：`user_dict.txt` 是源，`user.bin` 是编译产物。
/// 用户手改了 txt 却没重新编译时，如果还拿 bin 去查，表现就是「改了不生效」，
/// 这种静默失效最难查。所以宁可退回慢一点的 txt 覆盖层。
fn probe_user_bin() -> Option<&'static Bin> {
    let bin = user_bin_file();
    let txt = user_file();

    let bin_time = mtime(&bin)?;
    if let Some(txt_time) = mtime(&txt) {
        if txt_time > bin_time {
            return None;
        }
    }
    // 文件在但读不出来（格式坏、被截断、版本不对）也退回 txt
    dictbin::map_file(&bin)
}

/// 当前生效的 `user.bin`。第一次调用时探测，之后零开销。
fn user_bin() -> Option<&'static Bin> {
    if let Ok(g) = USER_BIN.read() {
        if g.probed {
            return g.bin;
        }
    }
    refresh_user_bin();
    USER_BIN.read().ok().and_then(|g| g.bin)
}

/// 重新探测 `user.bin`。
///
/// 别的进程重编译了 `user.bin` 之后，本进程还映射着旧文件（旧映射不会失效，
/// 但看不到新词）。在 `Activate` 里调一次，代价是两次 `GetFileAttributesEx`。
pub fn refresh_user_bin() {
    let loaded = probe_user_bin();
    if let Ok(mut g) = USER_BIN.write() {
        g.probed = true;
        g.bin = loaded;
    }
}

/// 当前生效的用户词库来源
enum UserSrc {
    /// 没有用户词库
    None,
    /// 常态：mmap 进来的 user.bin
    Mapped(&'static Bin),
    /// 回退：解析 user_dict.txt 得到的堆索引
    Heap(Arc<Overlay>),
}

fn user_source() -> UserSrc {
    if let Some(b) = user_bin() {
        return UserSrc::Mapped(b);
    }
    let ov = overlay();
    if ov.entries.is_empty() {
        UserSrc::None
    } else {
        UserSrc::Heap(ov)
    }
}

/// 把 `user_dict.txt` 编译成 `user.bin`，然后立刻重新映射。
///
/// 返回 `(条目数, 字节数)`。写临时文件再改名，避免编译到一半时别的进程
/// 映射到半截文件；改名后旧映射仍然指向旧文件（Windows 允许改名已映射的文件）。
pub fn compile_user_bin() -> Result<(usize, usize), String> {
    let txt = std::fs::read_to_string(user_file()).map_err(|e| format!("读不到 user_dict.txt：{e}"))?;
    let entries = dictenc::parse_entries(&txt);
    if entries.is_empty() {
        return Err("user_dict.txt 里没有有效条目（格式：每行「词<TAB>全拼」）".into());
    }
    let built = dictenc::build(&entries);

    std::fs::create_dir_all(config::dir()).map_err(|e| format!("建目录失败：{e}"))?;
    let path = user_bin_file();
    let tmp = path.with_extension("bin.tmp");
    std::fs::write(&tmp, &built.bytes).map_err(|e| format!("写 user.bin.tmp 失败：{e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("替换 user.bin 失败（可能被别的进程占用）：{e}")
    })?;

    refresh_user_bin();
    Ok((built.count, built.bytes.len()))
}

static OVERLAY: RwLock<Option<Arc<Overlay>>> = RwLock::new(None);

fn overlay() -> Arc<Overlay> {
    if let Ok(g) = OVERLAY.read() {
        if let Some(o) = g.as_ref() {
            return o.clone();
        }
    }
    let built = Arc::new(Overlay::load());
    if let Ok(mut g) = OVERLAY.write() {
        *g = Some(built.clone());
    }
    built
}

fn reload_overlay() {
    let built = Arc::new(Overlay::load());
    if let Ok(mut g) = OVERLAY.write() {
        *g = Some(built);
    }
}

// ─────────────────────────────────────────────────────────────
// 查询
// ─────────────────────────────────────────────────────────────

/// 运行时导入的用户词库文件
pub fn user_file() -> PathBuf {
    config::dir().join("user_dict.txt")
}

/// 按双拼编码前缀查候选，最多返回 `limit` 条。
/// 顺序即「覆盖层优先、词频降序」。
pub fn lookup(prefix: &str, limit: usize) -> Vec<Cow<'static, str>> {
    search(false, prefix, "", AssistMode::Off, limit)
}

/// 带辅助码筛选的双拼查询。
///
/// `assist_prefix` 为空时等价于 `lookup`。非空时只保留**首字辅助码**以它开头的候选
/// （音形方案里词组就按首字筛选，和主流输入法一致）。
pub fn lookup_filtered(
    prefix: &str,
    assist_prefix: &str,
    mode: AssistMode,
    limit: usize,
) -> Vec<Cow<'static, str>> {
    search(false, prefix, assist_prefix, mode, limit)
}

/// 全拼模式查询（前缀是「全拼去掉空格」后的串，例如 `nihao`）
#[allow(dead_code)]
pub fn lookup_pinyin(prefix: &str, limit: usize) -> Vec<Cow<'static, str>> {
    search(true, prefix, "", AssistMode::Off, limit)
}

/// 全拼模式 + 辅助码筛选
pub fn lookup_pinyin_filtered(
    prefix: &str,
    assist_prefix: &str,
    mode: AssistMode,
    limit: usize,
) -> Vec<Cow<'static, str>> {
    search(true, prefix, assist_prefix, mode, limit)
}

fn search(
    pinyin: bool,
    prefix: &str,
    assist_prefix: &str,
    mode: AssistMode,
    limit: usize,
) -> Vec<Cow<'static, str>> {
    let mut out = collect(pinyin, prefix, assist_prefix, mode, limit);
    // 动态词频只在这一层做：双拼/全拼、带不带辅助码的查询全都汇到这里，
    // 不会出现「某个入口忘了排序」这种只在特定模式下复现的怪事。
    //
    // 注意排序发生在**截断之后**：已经按 limit 收满的候选才会被重排。
    // 所以学习记录只能把「本来就在候选列表里」的词往前挪，不能把排到
    // 90 名以外的词捞进来。这个限制是刻意的——真去全量扫描会让每次按键
    // 都多扫几万条，换来的收益是「某个极冷僻词被选过一次就能上第一页」，
    // 不值。
    crate::freq::rank(&mut out);
    out
}

/// 按「用户词库优先 → 静态词库」的顺序收集候选，不做动态排序。
fn collect(
    pinyin: bool,
    prefix: &str,
    assist_prefix: &str,
    mode: AssistMode,
    limit: usize,
) -> Vec<Cow<'static, str>> {
    if prefix.is_empty() || limit == 0 {
        return Vec::new();
    }
    let which = if pinyin {
        Idx::Flat
    } else {
        match scheme() {
            Scheme::Xiaohe => Idx::Xiaohe,
            Scheme::Ms => Idx::Ms,
        }
    };

    let mut out: Vec<Cow<'static, str>> = Vec::with_capacity(limit.min(16));
    // 去重用的短列表：候选最多 limit 条，线性扫比建 HashSet 划算（也免了分配）
    let mut seen: Vec<&str> = Vec::with_capacity(limit.min(16));

    let pass = |w: &str| -> bool {
        assist_prefix.is_empty()
            || assist::code_of_word(w, mode).is_some_and(|c| c.starts_with(assist_prefix))
    };

    // 1) 用户词库优先。常态是 mmap 的 user.bin（和 base.bin 同一条零解析路径，
    //    多进程共享页缓存）；只有 bin 缺失或比 txt 旧时才退回堆索引。
    //
    // `user` 必须在 match 之外绑定：`seen` 借用它的字符串，作用域要比 match 长。
    let user = user_source();
    match &user {
        UserSrc::Mapped(b) => {
            let (ps, pe) = b.prefix_range(which, prefix);
            let ee = b.exact_end(which, prefix).clamp(ps, pe);
            for (a, z) in [(ps, ee), (ee, pe)] {
                for p in a..z {
                    let w = b.word_at(which, p);
                    if !pass(w) || seen.contains(&w) {
                        continue;
                    }
                    seen.push(w);
                    out.push(Cow::Borrowed(w));
                    if out.len() >= limit {
                        return out;
                    }
                }
            }
        }
        UserSrc::Heap(ov) => {
            let (ps, pe) = ov.prefix_range(which, prefix);
            let ee = ov.exact_end(which, prefix).clamp(ps, pe);
            // 整码匹配先出，再出更长的词
            for (a, b) in [(ps, ee), (ee, pe)] {
                for p in a..b {
                    let w = ov.word_at(which, p);
                    if !pass(w) || seen.contains(&w) {
                        continue;
                    }
                    seen.push(w);
                    out.push(Cow::Owned(w.to_string()));
                    if out.len() >= limit {
                        return out;
                    }
                }
            }
        }
        UserSrc::None => {}
    }

    // 2) 静态词库
    let bin = dictbin::get();
    {
        let (ps, pe) = bin.prefix_range(which, prefix);
        let ee = bin.exact_end(which, prefix).clamp(ps, pe);
        for (a, b) in [(ps, ee), (ee, pe)] {
            for p in a..b {
                let w = bin.word_at(which, p);
                if !pass(w) || seen.contains(&w) {
                    continue;
                }
                seen.push(w);
                out.push(Cow::Borrowed(w));
                if out.len() >= limit {
                    return out;
                }
            }
        }
    }
    out
}

/// 词库概况：(总条目, 用户词库条目, 当前方案)
pub fn stats() -> (usize, usize, Scheme) {
    let user = match user_source() {
        UserSrc::Mapped(b) => b.count(),
        UserSrc::Heap(o) => o.entries.len(),
        UserSrc::None => 0,
    };
    (dictbin::get().count() + user, user, scheme())
}

/// 导出全部词库到 `词<TAB>全拼` 文本。返回条目数。
pub fn export_to(path: &Path) -> Result<usize, String> {
    let bin = dictbin::get();
    let ov = overlay();
    let mut out = String::with_capacity((bin.count() + ov.entries.len()) * 24);
    out.push_str("# 飞鹤输入法词库导出\n# 格式：词<TAB>全拼（音节空格分隔）\n");

    for (w, py) in &ov.entries {
        out.push_str(w);
        out.push('\t');
        out.push_str(py);
        out.push('\n');
    }
    for i in 0..bin.count() {
        out.push_str(bin.word(i));
        out.push('\t');
        out.push_str(bin.pinyin(i));
        out.push('\n');
    }

    std::fs::write(path, out).map_err(|e| format!("写入失败：{e}"))?;
    Ok(bin.count() + ov.entries.len())
}

/// 从 `词<TAB>全拼` 文本导入，追加到运行时用户词库，并立刻编译成 `user.bin`。
///
/// 返回 `(新增条目数, Some((bin 条目数, bin 字节数)))`；第二个为 `None`
/// 表示编译失败、退回文本覆盖层（导入本身是成功的）。
pub fn import_from(path: &Path) -> Result<(usize, Option<(usize, usize)>), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("读取失败：{e}"))?;

    let existing = std::fs::read_to_string(user_file()).unwrap_or_default();
    let mut known: HashSet<String> = existing
        .lines()
        .filter_map(|l| l.split_once('\t').map(|(w, _)| w.trim().to_string()))
        .collect();

    let mut appended = String::new();
    let mut added = 0usize;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((word, pinyin)) = line.split_once('\t') else {
            continue;
        };
        let (word, pinyin) = (word.trim(), pinyin.trim());
        if word.is_empty() || pinyin.is_empty() {
            continue;
        }
        // 每个音节都得能编码，否则这行是脏数据
        if pinyin.split_whitespace().any(|s| {
            shuangpin::encode_syllable(Scheme::Xiaohe, s).is_none()
                && shuangpin::encode_syllable(Scheme::Ms, s).is_none()
        }) {
            continue;
        }
        if !known.insert(word.to_string()) {
            continue;
        }
        appended.push_str(word);
        appended.push('\t');
        appended.push_str(pinyin);
        appended.push('\n');
        added += 1;
    }

    if added > 0 {
        let mut all = existing;
        if !all.is_empty() && !all.ends_with('\n') {
            all.push('\n');
        }
        all.push_str(&appended);
        let _ = std::fs::create_dir_all(config::dir());
        std::fs::write(user_file(), all).map_err(|e| format!("写入失败：{e}"))?;

        // 导入完立刻编译成 user.bin，让用户词库走 mmap 而不是每进程解析一遍。
        // 编译失败不算导入失败——txt 已经写好了，退回覆盖层照样能用，只是慢一点。
        match compile_user_bin() {
            Ok((n, bytes)) => return Ok((added, Some((n, bytes)))),
            Err(_) => reload_overlay(),
        }
    }
    Ok((added, None))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex;

    /// `SCHEME` / `pinyin_mode` 都是进程级全局状态，测试并行跑会互相篡改。
    /// 凡是碰这两个状态的用例（包括其它模块的）统一拿这把锁串行。
    pub(crate) static SCHEME_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn dict_not_empty() {
        let (total, _, _) = stats();
        assert!(total > 1000, "dict too small: {total}");
    }

    #[test]
    fn lookup_prefix_hits() {
        let _g = SCHEME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_scheme(Scheme::Xiaohe);
        // nihc = ni hao（小鹤）
        let r = lookup("nihc", 10);
        assert!(!r.is_empty(), "nihc should have candidates");
        assert!(r.iter().any(|w| &**w == "你好"), "expected 你好 in {r:?}");
    }

    #[test]
    fn lookup_is_ordered_by_frequency() {
        let _g = SCHEME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_scheme(Scheme::Xiaohe);
        let r = lookup("de", 5);
        assert_eq!(
            r.first().map(|s| &**s),
            Some("的"),
            "expected 的 first, got {r:?}"
        );
    }

    #[test]
    fn lookup_respects_limit() {
        assert!(lookup("a", 3).len() <= 3);
    }

    #[test]
    fn retroflex_lookup() {
        let _g = SCHEME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_scheme(Scheme::Xiaohe);
        // 小鹤：shi = ui、zhong = vs
        assert!(lookup("ui", 5).iter().any(|w| &**w == "是"));
        assert!(lookup("vs", 5).iter().any(|w| &**w == "中"));
    }

    #[test]
    fn ms_scheme_lookup() {
        let _g = SCHEME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // 微软：ying = y;（ing 在分号键）
        set_scheme(Scheme::Ms);
        let r = lookup("y;", 10);
        set_scheme(Scheme::Xiaohe);
        assert!(!r.is_empty(), "y; should match in Ms scheme");
    }

    #[test]
    fn no_duplicate_words_in_lookup() {
        let r = lookup("nihc", 90);
        let mut uniq = r.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(uniq.len(), r.len(), "duplicate candidates: {r:?}");
    }

    #[test]
    fn assist_filter_narrows() {
        let _g = SCHEME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // 小鹤 de 有 的/得/地/德…；自然码辅助码里 的→白→b，得/德→彳→c，地→土→t。
        // 注意 limit 必须足够大，否则两边都被截到 limit 条，长度比较没有意义。
        let all = lookup("de", 5000);
        let filtered = lookup_filtered("de", "b", AssistMode::Ziranma, 5000);

        assert!(!filtered.is_empty(), "assist filter should not empty out");
        assert!(filtered.len() < all.len(), "assist filter should narrow");
        assert!(
            filtered.iter().any(|w| &**w == "的"),
            "expected 的 in {:?}",
            &filtered[..filtered.len().min(10)]
        );
        // 留下的必须全部是「首字辅助码以 b 开头」的词
        assert!(
            filtered.iter().all(|w| assist::code_of_word(w, AssistMode::Ziranma)
                .is_some_and(|c| c.starts_with('b'))),
            "filter leaked non-b candidates"
        );
        // 不带辅助码时结果必须和 lookup 完全一致
        assert_eq!(
            lookup_filtered("de", "", AssistMode::Off, 5000).len(),
            all.len()
        );
        // 小鹤音形：的→bs，用 bs 精确筛只剩「的」系
        let xh = lookup_filtered("de", "bs", AssistMode::Xiaohe, 5000);
        assert!(xh.iter().any(|w| &**w == "的"), "expected 的 in xiaohe {xh:?}");
        assert!(xh.len() < all.len());
    }

    #[test]
    fn pinyin_lookup_works() {
        let _g = SCHEME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // 全拼模式：直接敲 nihao / zhongguo
        let r = lookup_pinyin("nihao", 10);
        assert!(!r.is_empty(), "nihao should have candidates");
        assert!(r.iter().any(|w| &**w == "你好"), "expected 你好 in {r:?}");

        let r = lookup_pinyin("zhongguo", 10);
        assert!(r.iter().any(|w| &**w == "中国"), "expected 中国 in {r:?}");

        // 前缀：ni 应该能出以 ni 开头的词
        let r = lookup_pinyin("ni", 10);
        assert!(r.iter().any(|w| w.starts_with('你')), "expected 你* in {r:?}");

        // 全拼模式下不按双拼码解释：ui 在全拼里没有对应音节
        assert!(lookup_pinyin("ui", 10).is_empty(), "ui is not a pinyin prefix");
    }

    #[test]
    fn overlay_overrides_static() {
        // 构造一个覆盖层，验证它排在静态词库前面且能盖住同名词
        let ov = Overlay::parse("你好\tni hao\n飞鹤\tfei he\n");
        assert_eq!(ov.entries.len(), 2);
        let hits: Vec<_> = ov.prefix(Idx::Xiaohe, "nihc").collect();
        assert_eq!(hits, vec!["你好"]);
        let hits: Vec<_> = ov.prefix(Idx::Flat, "feihe").collect();
        assert_eq!(hits, vec!["飞鹤"]);
    }

    /// 量一下「把整个个人词库当覆盖层加载」的代价。
    ///
    /// 覆盖层是**每进程私有堆**：这 5 万条会被每个加载了飞鹤的进程各解析一遍，
    /// 和静态部分的「共享只读页」不是一回事。要给出这个数量级的具体数字，
    /// 光靠猜不行，所以留一个手工跑的口子。
    ///
    /// `cargo test --release -- --ignored overlay_cost --nocapture`
    #[test]
    #[ignore = "手工测量用，依赖 src/user_dict.txt"]
    fn overlay_cost() {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/user_dict.txt");
        let Ok(text) = std::fs::read_to_string(&p) else {
            return;
        };
        let t = std::time::Instant::now();
        let ov = Overlay::parse(&text);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let payload: usize = ov.entries.iter().map(|(w, py)| w.len() + py.len()).sum();
        // 三张码表各有一份 Box<str>，真实堆占用 ≈ 净载荷 × 3 + 每条约 32B 的分配头
        let allocs = ov.entries.len() * 5; // 2 个词面/拼音 + 3 张表
        println!(
            "覆盖层：{} 条 / 解析 {:.1} ms / 净载荷 {:.2} MB / 约 {} 次小分配",
            ov.entries.len(),
            ms,
            payload as f64 / 1048576.0,
            allocs
        );
    }

    #[test]
    fn exact_code_beats_longer() {
        let _g = SCHEME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // 打满一个词的码时，整码匹配必须排在更长的词前面。
        // iuliqi = chu li qi = 处理器；词库里还有「处理器吧」(chu li qi ba)，
        // 它来自搜狗用户词库，词频更高，如果不做整码优先就会抢走第一位。
        set_scheme(Scheme::Xiaohe);
        let r = lookup("iuliqi", 5);
        assert_eq!(
            r.first().map(|s| &**s),
            Some("处理器"),
            "expected 处理器 first, got {r:?}"
        );

        // 另一组：de 的整码候选（的/得/地…）必须都在「的确」这类更长的词前面
        let r = lookup("de", 10);
        assert_eq!(r.first().map(|s| &**s), Some("的"), "got {r:?}");
    }

    #[test]
    fn extra_dict_is_merged() {
        let _g = SCHEME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // 补充表里的词要能查到（处理器不在 rime-ice 的 base 词表里）
        set_scheme(Scheme::Xiaohe);
        assert!(lookup("iuliqi", 90).iter().any(|w| &**w == "处理器"));
        // 全拼模式同样能查到
        assert!(lookup_pinyin("chuliqi", 90).iter().any(|w| &**w == "处理器"));
    }

    /// 首次查询的耗时直接决定第一次按键的手感。
    /// 单独跑：cargo test --release perf_first_lookup -- --nocapture --test-threads=1
    #[test]
    fn perf_first_lookup() {
        let _g = SCHEME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_scheme(Scheme::Xiaohe);
        let t0 = std::time::Instant::now();
        let r = lookup("n", 90);
        let first = t0.elapsed();

        let t1 = std::time::Instant::now();
        for _ in 0..1000 {
            let _ = lookup("nih", 90);
        }
        let per = t1.elapsed() / 1000;

        println!(
            "entries={}  first_lookup={:?}  subsequent={:?}",
            stats().0,
            first,
            per
        );
        assert!(!r.is_empty());
    }
}
