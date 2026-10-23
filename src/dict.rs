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

    fn word_at(&self, which: Idx, p: usize) -> &str {
        self.entries[self.table(which)[p].1 as usize].0.as_ref()
    }

    /// 前缀排序键：`(码长, entries 下标)`。语义同 `Bin::rank_key`。
    fn rank_key(&self, which: Idx, p: usize) -> (usize, usize) {
        let (code, i) = &self.table(which)[p];
        (code.len(), *i as usize)
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

/// 「编译成二进制」的结果。
pub enum CompileOutcome {
    /// 真的编译了
    Compiled { count: usize, bytes: usize },
    /// 没有 `user_dict.txt`（全新安装、从没导入过个人词库）。
    /// 这种情况本来就无事可做，不是错误——`existing` 是现有 `user.bin` 的条目数。
    NoSource { existing: Option<usize> },
}

/// 把 `user_dict.txt` 编译成 `user.bin`，然后立刻重新映射。
///
/// 写临时文件再改名，避免编译到一半时别的进程映射到半截文件；改名后旧映射
/// 仍然指向旧文件（Windows 允许改名已映射的文件）。
///
/// **源文件不存在不算失败**：全新安装没有 `user_dict.txt`，用户点按钮只是想
/// 「把词库整理一下」，这时候报「系统找不到指定的文件」纯属吓人。返回
/// `NoSource`，由调用方给一句人话。
pub fn compile_user_bin() -> Result<CompileOutcome, String> {
    let src = user_file();
    let txt = match std::fs::read_to_string(&src) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(CompileOutcome::NoSource {
                existing: dictbin::count_file(&user_bin_file()),
            })
        }
        Err(e) => return Err(format!("读不到 {}：{e}", src.display())),
    };
    let entries = dictenc::parse_entries(&txt);
    if entries.is_empty() {
        return Err("user_dict.txt 是空的（格式：每行「词<TAB>全拼」），没有可编译的内容".into());
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
    Ok(CompileOutcome::Compiled {
        count: built.count,
        bytes: built.bytes.len(),
    })
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
    let (mut out, lens) = collect(pinyin, prefix, assist_prefix, mode, limit);
    // 动态词频只在这一层做：双拼/全拼、带不带辅助码的查询全都汇到这里，
    // 不会出现「某个入口忘了排序」这种只在特定模式下复现的怪事。
    //
    // 注意排序发生在**截断之后**：已经按 limit 收满的候选才会被重排。
    // 所以学习记录只能把「本来就在候选列表里」的词往前挪，不能把排到
    // 90 名以外的词捞进来。这个限制是刻意的——真去全量扫描会让每次按键
    // 都多扫几万条，换来的收益是「某个极冷僻词被选过一次就能上第一页」，
    // 不值。
    crate::freq::rank(&mut out, &lens);

    // 自造词组：用户一段一段拼出来的、词库里本来没有的词（`limkyr`→李明远）。
    // 码打满就出，并且排在最前面——这是用户刚造的词，他下次敲这个码要的就是它。
    //
    // 位置在「一简字」和「置顶」**之前**，也就是它们都还压在上面：一简字是
    // 打字人的肌肉记忆，置顶是用户显式钉死的，两者都该赢过自动学来的东西。
    //
    // 带辅助码时不参与：辅助码是用户主动「再筛一道」，可自造词没有可靠的首字
    // 辅助码，硬塞进去等于把用户的筛选当没发生。
    if assist_prefix.is_empty() {
        // `words_for` 给的是「新学的在前」；倒着 promote（每个都插到 0 号位）
        // 才能让最新的那个最后落到最前面。
        for w in crate::phrase::words_for(prefix, pinyin).into_iter().rev() {
            promote(&mut out, &w, true, limit);
        }
    }

    // 二简词：两键出一个常用两字词（`nd` 难道、`sy` 所以）。小鹤官方「二简」，
    // 见 `brief2`。
    //
    // 只在小鹤方案下生效：表里的码是按小鹤键位算的，微软双拼的零声母用 `o` 占位，
    // 同一串两键码指向的音节完全不同，套上去等于乱给候选。
    //
    // 带辅助码筛选时也不参与，理由同自造词：用户已经主动筛了一道，而二简词没有
    // 可靠的首字辅助码可对。
    //
    // 首选提到 0 号位（哪怕候选里本来没有它），次选放在 1 号位——次选的存在前提
    // 就是「这个码本来归某个更常用的单字」，抢了 0 号位就跟首选撞车了。
    // 置顶仍在最后执行，所以用户显式钉的词照样赢过二简词。
    if !pinyin && assist_prefix.is_empty() && scheme() == Scheme::Xiaohe {
        if let Some(w) = crate::brief2::word_for(prefix) {
            promote(&mut out, w, true, limit);
        }
        if let Some(w) = crate::brief2::alt_for(prefix) {
            place(&mut out, w, 1, limit);
        }
    }

    // 一简字：单键查询时把「这个键该出的字」提到第一位。
    //
    // 放在 `freq::rank` **之后**，所以一简字赢过动态词频——这是刻意的：
    // 一简字是打字人的肌肉记忆（`d` 后面按空格就该出「的」），不能被
    // 「我上次选过『大』」这种偶发学习顶掉。真想让别的字排第一，用置顶。
    //
    // `insert_missing = false`：候选里没有它就算了。微软双拼的键位和小鹤
    // 不同，开辅助码筛选时也可能把它筛掉——这两种情况下硬塞一个不相干的字
    // 比不生效更糟。
    if let Some(c) = crate::brief::char_for(prefix) {
        let mut buf = [0u8; 4];
        promote(&mut out, c.encode_utf8(&mut buf), false, limit);
    }

    // 一简次选：单键查询时把该键的次选字/词放到**第二位**（`d` → 的 / 打）。
    //
    // 只占 1 号位，不抢 0 号位——首位永远是首选一简字。这里允许插入：
    // 次选里有「嗯嗯」「可以」「词库」这类两字词，词库未必收，不插就永远不出现
    // （小鹤把它们放在独立的分类词库里，我们直接内嵌）。
    if let Some(w) = crate::brief::alt_for(prefix) {
        place(&mut out, w, 1, limit);
    }

    // 手动置顶压在最后，也就是排在最前：用户显式钉的东西必须赢过一切默认值。
    // 这里 `insert_missing = true`——置顶的全部意义就是「哪怕它不在候选里也要出」。
    if let Some(w) = crate::pin::word_for(prefix) {
        promote(&mut out, &w, true, limit);
    }
    out
}

/// 把 `word` 提到候选最前面。
///
/// - 已经在列表里：原地取出再插到 0 号位（保持其余顺序不变）
/// - 不在列表里且 `insert_missing`：直接插到最前，并把尾部挤出去，保持长度不变
fn promote(out: &mut Vec<Cow<'static, str>>, word: &str, insert_missing: bool, limit: usize) {
    if let Some(i) = out.iter().position(|w| w.as_ref() == word) {
        if i > 0 {
            let w = out.remove(i);
            out.insert(0, w);
        }
        return;
    }
    if !insert_missing || word.is_empty() {
        return;
    }
    out.insert(0, Cow::Owned(word.to_string()));
    out.truncate(limit.max(1));
}

/// 把 `word` 放到候选第 `pos` 位（越界就放到末尾）。不在列表里也插进去。
///
/// 和 `promote` 只差「放第几位」：次选二简词得排在该码的单字后面，不能抢 0 号位。
/// 必须允许插入——次选词是 `brief2.txt` 里列的，词库未必收，不插就永远不出现。
fn place(out: &mut Vec<Cow<'static, str>>, word: &str, pos: usize, limit: usize) {
    if word.is_empty() {
        return;
    }
    let at = pos.min(out.len());
    if let Some(i) = out.iter().position(|w| w.as_ref() == word) {
        let w = out.remove(i);
        out.insert(at.min(out.len()), w);
        return;
    }
    out.insert(at, Cow::Owned(word.to_string()));
    out.truncate(limit.max(1));
}

/// 给 `pin` 模块的单测用：验证提权语义，不必真去查词库。
#[cfg(test)]
pub(crate) fn promote_for_test(
    out: &mut Vec<Cow<'static, str>>,
    word: &str,
    insert_missing: bool,
    limit: usize,
) {
    promote(out, word, insert_missing, limit);
}

/// 反查「某个编码下的某个词」的全拼。整码匹配，不做前缀。
///
/// 只给自动记词组用（`textservice::remember_phrase`）：分词上屏时每一段都知道
/// 「码 + 选中的词」，要把它记成新词就得知道这个词的全拼，而词库里存的正是全拼。
///
/// 为什么必须整码：`li` 这个前缀下还挂着 `limk`（李明）这类更长的条目，只按前缀
/// 扫会把长词的全拼套到当前这个词上，记下来的词下次就打不出来了（码对不上）。
pub fn pinyin_of(code: &str, word: &str, pinyin: bool) -> Option<String> {
    if code.is_empty() || word.is_empty() {
        return None;
    }
    let which = if pinyin {
        Idx::Flat
    } else {
        match scheme() {
            Scheme::Xiaohe => Idx::Xiaohe,
            Scheme::Ms => Idx::Ms,
        }
    };

    let user = user_source();
    match &user {
        UserSrc::Mapped(b) => {
            let (ps, pe) = b.prefix_range(which, code);
            for p in ps..pe {
                if b.rank_key(which, p).0 == code.len() && b.word_at(which, p) == word {
                    return Some(b.pinyin(b.raw_of(which, p)).to_string());
                }
            }
        }
        UserSrc::Heap(ov) => {
            let (ps, pe) = ov.prefix_range(which, code);
            for p in ps..pe {
                if ov.rank_key(which, p).0 == code.len() && ov.word_at(which, p) == word {
                    // entries 是 (词, 全拼)，这里要的是全拼
                    return Some(ov.entries[ov.table(which)[p].1 as usize].1.to_string());
                }
            }
        }
        UserSrc::None => {}
    }

    let bin = dictbin::get();
    let (ps, pe) = bin.prefix_range(which, code);
    for p in ps..pe {
        if bin.rank_key(which, p).0 == code.len() && bin.word_at(which, p) == word {
            return Some(bin.pinyin(bin.raw_of(which, p)).to_string());
        }
    }
    None
}

/// 前缀区间里挑出排在最前面的 `limit` 个位置，键是 `(码长, 原始行号)`。
///
/// ## 为什么不能只按码排
///
/// 索引表是按**码**升序排的。整码查询（`nihc` → 你好）没问题，但前缀查询会跨
/// 好几个音节，按码排就等于按字母排——`d` 的 18 个候选会全是 `da` 那一批
/// （大/打/答/达…），`de` 的「的」永远进不来，单键打字直接废掉。
///
/// ## 为什么键里要带码长
///
/// 只按行号排也不行：行号是「词库文件里的第几行」，而 `base.bin` 是
/// 「个人词库 → rime-ice → 补充表」拼起来的，个人词库那一段排在前面，
/// 于是 `d` 的前 18 名会变成「都是 / 对了 / 都有」这种两字词，单字反而一个不剩。
///
/// 码长短 = 码更「完整」：单键 `d` 下的两码条目正好是一个整音节，四码条目是
/// 两个音节组成的词。先按码长分组、组内再按词频，就同时得到两条正确的语义：
///
/// - 打 `d`：只出单字，且按词频排（的/大/到/都…）
/// - 打 `nihc`：整码的「你好」排在「你好吗」这类更长的词前面
///
/// 这正好是原来「整码优先」那套规则的推广——原来用二分找出「码恰好等于前缀」
/// 的一段，现在变成「码长等于最短那一档」，语义一致且顺带修好了单键。
///
/// ## 为什么不整段排序
///
/// 区间可能有几千条，而我们只要最前面的十几个。这里维护一个长度 ≤ `limit` 的
/// 升序小表，绝大多数条目一次比较就被 `continue` 掉，代价是「扫一遍区间」
/// 而不是「排一遍区间」。
fn top_by_rank(
    range: std::ops::Range<usize>,
    limit: usize,
    key_of: impl Fn(usize) -> (usize, usize),
) -> Vec<usize> {
    let mut best: Vec<((usize, usize), usize)> = Vec::with_capacity(limit.min(64));
    for p in range {
        let key = key_of(p);
        if best.len() >= limit && key >= best[best.len() - 1].0 {
            continue;
        }
        let pos = best.partition_point(|(k, _)| *k < key);
        best.insert(pos, (key, p));
        if best.len() > limit {
            best.pop();
        }
    }
    best.into_iter().map(|(_, p)| p).collect()
}

/// 按「用户词库优先 → 静态词库」的顺序收集候选，不做动态排序。
///
/// 每个来源内部走同一套排序键 `(码长, 词频行号)`，见 `top_by_rank`。
fn collect(
    pinyin: bool,
    prefix: &str,
    assist_prefix: &str,
    mode: AssistMode,
    limit: usize,
) -> (Vec<Cow<'static, str>>, Vec<u8>) {
    if prefix.is_empty() || limit == 0 {
        return (Vec::new(), Vec::new());
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
    // 每个候选的码长，和 `out` 一一对应。动态词频排序要用它守住「短码 = 更完整」，
    // 否则学过一次「程序」就会把 `ig` 的「成」挤下去（程序只是前缀匹配，4 键）。
    let mut lens: Vec<u8> = Vec::with_capacity(limit.min(16));
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
            for p in top_by_rank(ps..pe, limit, |p| b.rank_key(which, p)) {
                let w = b.word_at(which, p);
                if !pass(w) || seen.contains(&w) {
                    continue;
                }
                seen.push(w);
                out.push(Cow::Borrowed(w));
                lens.push(b.rank_key(which, p).0.min(u8::MAX as usize) as u8);
                if out.len() >= limit {
                    return (out, lens);
                }
            }
        }
        UserSrc::Heap(ov) => {
            let (ps, pe) = ov.prefix_range(which, prefix);
            for p in top_by_rank(ps..pe, limit, |p| ov.rank_key(which, p)) {
                let w = ov.word_at(which, p);
                if !pass(w) || seen.contains(&w) {
                    continue;
                }
                seen.push(w);
                out.push(Cow::Owned(w.to_string()));
                lens.push(ov.rank_key(which, p).0.min(u8::MAX as usize) as u8);
                if out.len() >= limit {
                    return (out, lens);
                }
            }
        }
        UserSrc::None => {}
    }

    // 2) 静态词库
    let bin = dictbin::get();
    {
        let (ps, pe) = bin.prefix_range(which, prefix);
        for p in top_by_rank(ps..pe, limit, |p| bin.rank_key(which, p)) {
            let w = bin.word_at(which, p);
            if !pass(w) || seen.contains(&w) {
                continue;
            }
            seen.push(w);
            out.push(Cow::Borrowed(w));
            lens.push(bin.rank_key(which, p).0.min(u8::MAX as usize) as u8);
            if out.len() >= limit {
                return (out, lens);
            }
        }
    }
    (out, lens)
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
            Ok(CompileOutcome::Compiled { count, bytes }) => {
                return Ok((added, Some((count, bytes))))
            }
            // 刚写完 txt，不可能走到 NoSource；真走到也只当编译失败处理
            _ => reload_overlay(),
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

    /// 自造词必须排第一：用户上次一段一段拼出来的词，这次敲满同样的码就该直接给。
    /// 这条是「我刚组出来的词怎么又没了」的回归测试。
    ///
    /// 用编出来的词而不是真词库里的词：单测不能依赖本地词库里恰好没有哪个词
    /// （词库一改断言就飘，见项目约定）。
    #[test]
    fn self_made_phrase_comes_first() {
        let _g = SCHEME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_scheme(Scheme::Xiaohe);
        let word = "飞鹤自造词试";
        let py = "fei he zi zao ci shi";
        let code = shuangpin::encode_pinyin(Scheme::Xiaohe, py).expect("should encode");
        crate::phrase::learn(word, py);

        let r = lookup(&code, 90);
        assert_eq!(r.first().map(|s| &**s), Some(word), "got {r:?}");
        // 存的是全拼不是双拼码，所以换成全拼模式照样打得出来
        let r = lookup_pinyin("feihezizaocishi", 90);
        assert_eq!(r.first().map(|s| &**s), Some(word), "got {r:?}");
    }


    /// 同一个码下可以挂多个自造词，最新的排最前。
    ///
    /// 这是「造了 蛋蛋堂 之后再造 弹弹堂 就不行了」的回归测试：学习路径本身一直
    /// 支持同码多词（`phrase::learn` 按**词**去重），真正堵住用户的是进不去分词
    /// 模式（见 `textservice::arrow_into_segments`）。这里守的是另一半：
    /// 学会之后两个词都得在候选里，而且新的在前。
    #[test]
    fn two_phrases_under_one_code_coexist() {
        let _g = SCHEME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_scheme(Scheme::Xiaohe);
        let py = "dan dan tang";
        let code = shuangpin::encode_pinyin(Scheme::Xiaohe, py).expect("should encode");
        crate::phrase::learn("蛋蛋堂", py);
        crate::phrase::learn("弹弹堂", py);

        let r = lookup(&code, 90);
        assert_eq!(r.first().map(|s| &**s), Some("弹弹堂"), "最新的在前：{r:?}");
        assert!(r.iter().any(|w| &**w == "蛋蛋堂"), "旧的不能丢：{r:?}");

        // 全拼索引也一样：两个词都认，新的在前
        let r = lookup_pinyin("dandantang", 90);
        let fresh = r.iter().position(|w| &**w == "弹弹堂");
        let old = r.iter().position(|w| &**w == "蛋蛋堂");
        assert!(fresh.is_some() && old.is_some(), "got {r:?}");
        assert!(fresh < old, "新的在前：{r:?}");
    }

    /// 二简词（`brief2`）：两键出常用两字词。
    ///
    /// 三条语义各守一条：
    /// 1. 首选词**压过同码的单字**——`nd` 出「难道」，「奶」退到第二位
    /// 2. 次选词**排在该码的单字之后**——`mm` 仍是「面」打头，「慢慢」占第二位
    /// 3. 换方案就失效——微软双拼的两键码含义不同，不能套小鹤的表
    #[test]
    fn brief2_words_are_promoted() {
        let _g = SCHEME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_scheme(Scheme::Xiaohe);

        let r = lookup("nd", 10);
        assert_eq!(r.first().map(|s| &**s), Some("难道"), "got {r:?}");
        assert!(r.iter().any(|w| &**w == "奶"), "同码单字不能被挤掉：{r:?}");

        let r = lookup("mm", 10);
        assert_eq!(r.first().map(|s| &**s), Some("面"), "次选不该抢 0 号位：{r:?}");
        assert_eq!(r.get(1).map(|s| &**s), Some("慢慢"), "got {r:?}");

        // 首选 + 次选同码时占满前两位
        let r = lookup("vd", 10);
        assert_eq!(r.first().map(|s| &**s), Some("知道"), "got {r:?}");
        assert_eq!(r.get(1).map(|s| &**s), Some("直到"), "got {r:?}");

        // 微软双拼不查这张表
        set_scheme(Scheme::Ms);
        let r = lookup("nd", 10);
        assert!(r.iter().all(|w| &**w != "难道"), "微软方案不该出二简词：{r:?}");
        set_scheme(Scheme::Xiaohe);
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

    /// 一简字必须排第一：`d` 后面按空格出「的」，这是小鹤用户的肌肉记忆。
    ///
    /// 26 个键全部要过。这条同时守两个东西：
    ///
    /// 1. **提权接上了**——`d` 开头语料里「大」更靠前，不提权就会出「大」
    /// 2. **单键候选里真的有单字**——提权只做「候选里本来就有它才提前」，
    ///    如果单键查询退化成「只出两字词」，26 个一简字会集体消失。
    ///    这一条正是 `top_by_rank` 按码长分组要保证的
    #[test]
    fn every_brief_char_ranks_first() {
        let _g = SCHEME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_scheme(Scheme::Xiaohe);
        let mut bad = Vec::new();
        for (key, want) in crate::brief::TABLE {
            let r = lookup(&key.to_string(), 18);
            if r.first().map(|s| &**s) != Some(want.encode_utf8(&mut [0u8; 4])) {
                bad.push(format!("{key}→{want}: {r:?}"));
            }
        }
        assert!(bad.is_empty(), "这些键的一简字没排第一：\n{}", bad.join("\n"));
    }

    /// 一简次选：单键的第二位固定是该键的次选字/词（`d` → 的 / 打）。
    ///
    /// 首位永远是首选一简字，次选只占 1 号位——这是小鹤的「一简首选 / 次选」，
    /// 分号键 `d;` 就是直接取第二位（见 `textservice::brief_alt`）。
    #[test]
    fn brief_alt_ranks_second() {
        let _g = SCHEME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_scheme(Scheme::Xiaohe);
        let mut bad = Vec::new();
        for (key, want) in crate::brief::ALT {
            let r = lookup(&key.to_string(), 18);
            if r.get(1).map(|s| &**s) != Some(want) {
                bad.push(format!("{key}→{want}: {r:?}"));
            }
        }
        assert!(bad.is_empty(), "这些键的次选没排第二位：\n{}", bad.join("\n"));
    }

    /// 单键查询的候选：短码（单字）排在长码（多字词）前面。
    ///
    /// 用户打 `d` 想要的是「的/大/到」这一批单字，不是「都是 / 对了」。
    /// 两个例外，都是刻意塞进来的：
    ///
    /// - 1 号位是**一简次选**（`d` 的第二位固定是「打」、`c` 是「词库」）
    /// - 某些键的单字不够 18 个（`o` 只有「哦/噢/喔」这么几个），顺延到两字词更实用
    ///
    /// 所以断言的是**顺序**而不是「全部都是单字」：一旦出现词库里的多字词，
    /// 后面不能再冒出单字。次选那一条先挖掉，不参与这条顺序检查。
    #[test]
    fn single_key_yields_single_chars() {
        let _g = SCHEME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_scheme(Scheme::Xiaohe);
        for key in "abcdefghijklmnopqrstuvwxyz".chars() {
            let mut r = lookup(&key.to_string(), 18);
            assert!(!r.is_empty(), "`{key}` 应该有候选");
            if let Some(alt) = crate::brief::alt_for(&key.to_string()) {
                assert_eq!(r.get(1).map(|s| &**s), Some(alt), "`{key}` 次选位置不对：{r:?}");
                r.remove(1);
            }
            let mut word_seen = false;
            for (i, w) in r.iter().enumerate() {
                if w.chars().count() == 1 {
                    assert!(
                        !word_seen,
                        "`{key}` 第 {} 个是单字「{w}」，却排在了多字词后面：{r:?}",
                        i + 1
                    );
                } else {
                    word_seen = true;
                }
            }
            assert_eq!(
                r[0].chars().count(),
                1,
                "`{key}` 的第一个候选「{}」应该是单字",
                r[0]
            );
        }
    }

    /// 两键以上不该有一简字，顺序回到正常排序。
    #[test]
    fn brief_does_not_apply_to_longer_codes() {
        let _g = SCHEME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_scheme(Scheme::Xiaohe);
        // `de` 的候选是「的/得/地…」这一批字本身，第一个仍是「的」，
        // 但那是因为静态词频，不是因为一简字（`de` 没有一简字）。
        assert!(crate::brief::char_for("de").is_none());
        let r = lookup("de", 5);
        assert_eq!(r.first().map(|s| &**s), Some("的"));
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

        // 单键是**最贵**的一档：区间跨 20 多个音节、几千条，`top_by_rank` 要整段扫。
        // 必须钉住，否则「按码长分组」这个改动会在慢机器上把打字手感拖垮。
        let t2 = std::time::Instant::now();
        for _ in 0..1000 {
            let _ = lookup("d", 18);
        }
        let single = t2.elapsed() / 1000;

        println!(
            "entries={}  first_lookup={:?}  subsequent={:?}  single_key={:?}",
            stats().0,
            first,
            per,
            single
        );
        assert!(!r.is_empty());
    }
}
