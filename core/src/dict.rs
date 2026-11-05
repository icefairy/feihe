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
    // 临时名带进程 id：两个宿主进程同时编译时，固定的 `user.bin.tmp` 会被
    // 两边交叉写坏，rename 上去的是混合体（见 `config::tmp_path`）。
    let tmp = config::tmp_path(&path);
    std::fs::write(&tmp, &built.bytes).map_err(|e| format!("写 user.bin 临时文件失败：{e}"))?;
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

pub(crate) fn reload_overlay() {
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

// ─────────────────────────────────────────────────────────────
// 输入方式：按键串 → 要查的码
// ─────────────────────────────────────────────────────────────
//
// Windows 端自己管着「敲进来的键就是码」这件事（`textservice` 里一个
// `StringBuilder` 攒着，从不需要把全拼翻译成码）。Android 的 26 键键盘没有那个
// 前提：用户在手机上按的是字母，屏幕上没有「码」的概念，所以壳必须回答一个问题
// ——**这一串按键是双拼码，还是全拼？**
//
// 这个判断放在 core 里，不是因为它复杂，而是因为它是**第二份真值**的高危区：
// 判断规则要是写在 Kotlin 里，Windows 和 Android 出了分歧就只能靠人肉对。判断
// 本身是纯的（只依赖输入、方案、以及「这个码查得到东西吗」），所以能单测。

/// 输入串解析出来的「要查的码」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// 拿去查词库的码（双拼码；全拼模式下的串也会先被编成双拼码）。
    pub code: String,
    /// 这一串是**按全拼解释**出来的，而不是原样当码。
    /// 壳据此决定状态条上要不要显示 `全拼→码`。
    pub from_pinyin: bool,
}

/// 按键串 → 码。`probe` 回答「这个码查得到东西吗」。
///
/// ## 规则（顺序就是优先级）
///
/// - `pinyin_mode = true`（全拼模式）：**只有一种解释**。按键就是全拼，编不出码
///   就是没有。这里刻意不退回双拼——用户选了全拼，就是不想让 `xian` 出「西安」。
/// - `pinyin_mode = false`（双拼模式）：**先原样当码**，查得到就用；查不到才按全拼
///   解释。于是双拼模式实际是「双拼，且认全拼」，手机上第一次装完直接打 `nihao`
///   也能出字，不会因为默认值是双拼就一片空白。
///
/// ## 为什么抽成带探针的纯函数
///
/// 真探针是 `any`，它的答案依赖**词库内容**，而词库内容一律不许进断言（个人词库
/// 一导入断言当场不成立，这条在 `handoff` 里记过三次）。喂一个假探针，就能把
/// 「什么时候该退回全拼」钉死，和词库里有没有那个词完全无关。
pub fn resolve_code_with<F>(
    input: &str,
    pinyin_mode: bool,
    scheme: Scheme,
    probe: F,
) -> Option<Resolved>
where
    F: Fn(&str) -> bool,
{
    if input.is_empty() {
        return None;
    }
    if !pinyin_mode && probe(input) {
        return Some(Resolved {
            code: input.to_string(),
            from_pinyin: false,
        });
    }
    crate::shuangpin::encode_phrase(scheme, input).map(|code| Resolved {
        code,
        from_pinyin: true,
    })
}

/// 按键串 → （实际用的码, 候选）。Android 的 `lookupInputNative` 就是它。
///
/// 返回码是给状态条用的：打 `nihao` 时显示 `nihao→nihc`，让人看得见引擎在干什么；
/// 双拼模式下两者相同，就不显示箭头。
pub fn lookup_input(
    input: &str,
    pinyin_mode: bool,
    limit: usize,
) -> (String, Vec<Cow<'static, str>>) {
    if input.is_empty() || limit == 0 {
        return (String::new(), Vec::new());
    }
    match resolve_code_with(input, pinyin_mode, scheme(), any) {
        Some(r) => {
            // 码永远是双拼码，所以这里固定查双拼索引。全拼模式不查 `Idx::Flat`：
            // 同一批词在两套索引里的结果一模一样（双拼是音节的 1:1 重编码），
            // 而 Android 的 base.bin 不需要为了全拼多带一份索引。
            let mut words = search(false, &r.code, "", AssistMode::Off, limit);
            if words.is_empty() {
                // 整码查不到 → 试整句（`vhsjli` → 张三李）。**只在查不到时试**：
                // 整词优先，否则打 `nihc` 会先看到一堆拼凑的整句。
                if let Some(sentence) = compose_sentence(&r.code) {
                    words.push(Cow::Owned(sentence));
                }
            }
            (r.code, words)
        }
        None => {
            // 编不出码。全拼模式下这就等于「没有」；双拼模式下更可能是**一长串码**
            // （`vhsjli` 编不出任何拼音），那就交给分段——这正是手机上分词态缺的那块。
            if pinyin_mode {
                return (String::new(), Vec::new());
            }
            match compose_sentence(input) {
                Some(sentence) => (input.to_string(), vec![Cow::Owned(sentence)]),
                None => (String::new(), Vec::new()),
            }
        }
    }
}

/// 长串自动分词上屏：把双拼码切成几段，每段取第一名拼起来。
///
/// 手机上打 `vhsjli` 之前是空候选条，现在会给出「张三李」。分段规则和桌面端
/// **共用同一份**（`crate::segment::plan`）——这一点比结果好不好看更重要：两边
/// 各写一遍的话，同一个码在手机和桌面上切出不同的词，没人查得到原因。
pub fn compose_sentence(code: &str) -> Option<String> {
    compose_sentence_with(code, any_aligned, |c| first_aligned(c).map(|w| w.to_string()))
}

/// 一个双拼码覆盖几个音节（每音节恰好两键）。
fn syllables_of(code: &str) -> usize {
    code.len() / 2
}

/// 一次对齐扫描最多看多少条候选。
///
/// 看着小，其实是**下界**推出来的：双拼码长 = 2 × 音节数，而前缀匹配要求词码以
/// 段码开头，所以段码覆盖 n 个音节时，能匹配到的词**至少** n 个音节、n 个字——
/// 也就是说「字数对得上」的那一档恰好是码长最短的一档，`collect` 的排序
/// （码长升序）把它们放在最前面。留 12 条余量是给置顶/一简字那几道显式插入用的。
///
/// 这个值不能随便加大：`any_aligned` 在 DP 里会被调用 O(n²) 次，一次按键几十次
/// 查询，每条候选都要走一遍词频/降级/置顶流水线。
const ALIGN_SCAN: usize = 12;

/// 这个码下有没有**字数正好等于音节数**的候选（不认前缀）。
///
/// 和 `any` 的区别只有一个：**字数必须对齐**。分段时必须用它——`any` 认前缀，
/// 于是「张三李四」（码 `vhsjlisj`）会被当成 `vhsjli` 存在（用户只敲了三个音节），
/// DP 于是以为整串就是一个词，切分退化成逐字，整句变成「长三裏」。
///
/// 用「汉字个数」而不是「码长字段」判断：码长字段要从 `collect` 里透出来，
/// 而那条流水线后面还挂着一简字 / 二简词 / 短语 / 置顶一堆插入，长度数组会失配。
/// 汉字个数是候选自己的属性，改不到一半。
pub fn any_aligned(code: &str) -> bool {
    let want = syllables_of(code);
    if want == 0 {
        return false;
    }
    lookup(code, ALIGN_SCAN)
        .iter()
        .any(|w| w.chars().count() == want)
}

/// 这个码下**字数正好等于音节数**的第一名（不认前缀）。理由同 [`any_aligned`]。
pub fn first_aligned(code: &str) -> Option<Cow<'static, str>> {
    let want = syllables_of(code);
    if want == 0 {
        return None;
    }
    lookup(code, ALIGN_SCAN)
        .into_iter()
        .find(|w| w.chars().count() == want)
}

/// [`compose_sentence`] 的实现，两个词典操作从参数进来。
///
/// 抽成「带两个闭包的纯函数」和 `resolve_code_with` 同一个理由：真实现依赖词库
/// 内容，而词库内容一律不许进断言（个人词库一导入断言当场不成立）。
/// 喂假表就能把「怎么算一段、缺字时怎么办」钉死。
///
/// 任何一段查不到字就**整句放弃**：混着段码的「整句」（比如 `面ap`）上屏只会让人
/// 莫名其妙。宁可什么都不给，让用户接着打或按回车原样上屏。
pub fn compose_sentence_with(
    code: &str,
    has: impl Fn(&str) -> bool,
    first: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    // 一个音节谈不上分段。门槛放在 4（两个音节）上，顺带把「打一半」的串挡在外面。
    if code.len() < 4 {
        return None;
    }
    let segs = crate::segment::plan(code, false, has);
    if segs.len() < 2 {
        return None;
    }
    let mut out = String::with_capacity(segs.len() * 2);
    for s in &segs {
        out.push_str(&first(s)?);
    }
    Some(out)
}

/// 拼写纠错：整串零候选时的最后一招。
///
/// **为什么是独立函数、不并进 `search`**：`search` 是「查词库」，纠错是「查不到之后
/// 的补救」。混在一起的直接后果是 `lookup` 一个没有整词的码会返回纠错候选，于是
/// `textservice` 的自动分词（`maybe_split`，整串零候选时切段逐字组词）再也触发不了——
/// 用户打「安并」这种词库没收的词，会被塞一屏「按病 / 暗冰」之类的纠错候选，反而
/// 组不出他想要的字。
///
/// 所以调用顺序由 `textservice` 把关：正常查询 → 自动分词 → **还是空**才纠错。
/// 见 `correct` 的模块文档。
///
/// `pinyin` 为真（全拼模式）时直接返回空：纠错是按「每个音节恰好两键」切位置的，
/// 全拼里没有这个前提。
pub fn corrected(prefix: &str, pinyin: bool, limit: usize) -> Vec<Cow<'static, str>> {
    if pinyin || limit == 0 {
        return Vec::new();
    }
    let mut out: Vec<Cow<'static, str>> = Vec::new();
    for fix in crate::correct::variants(prefix, scheme()) {
        let (o, _) = collect(false, &fix.code, "", AssistMode::Off, 1);
        for w in o {
            if out.iter().any(|x| x.as_ref() == w.as_ref()) {
                continue;
            }
            out.push(w);
        }
        if out.len() >= limit {
            break;
        }
    }
    if out.len() > 1 {
        // 按「这个错码下选过几次」排：用户接受过一次纠错，下次同一个错码它就排前面。
        // 学的路径和别的候选完全一样（`commit_with` 记的是「错码 + 词」），
        // 不需要另开一张「纠错学习表」。码长都一样（就是输入本身），所以排序键
        // 退化成「次数」——0 次的保持生成顺序（稳定排序），也就是正确率顺序。
        let lens = vec![prefix.len().min(u8::MAX as usize) as u8; out.len()];
        let tiers = vec![0u8; out.len()];
        crate::freq::rank(&mut out, &lens, &tiers, prefix);
    }
    out
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

// ─────────────────────────────────────────────────────────────
// 探测：只问「有没有」和「第一名是什么」
// ─────────────────────────────────────────────────────────────
//
// 这两个问题**成本不同、答案也不同**，所以分成两组函数，不再用 `lookup(..., 1)` 一句
// 话敷衍过去：
//
// - `any_*`：**只要存在性**。存不存在和顺序、候选上限都无关，所以可以只让收集器
//   找一条就停——最便宜，判定「这个码成不成立」用它。
// - `first_*`：**要第一名**。第一名是完整流水线的产物（词库归并 → 动态词频置换 →
//   形码打满置顶 → 自造词），任何一步只看到一条都会算错。`ji` 就是现成的例子：
//   只取一条时是「级」，跑完词频置换后是「奇」——用户学过的词会被提上来。
//
// 为什么当初会出 `iam` 那次事故：判定路径图便宜用了 `limit = 1`，而收集器在那条
// 小 limit 上「用户词库满了就返回」，核心词库一条没扫，「查」根本不在候选里，
// 「第一名是单字」永远不成立，于是拒绝拆分、`iam` 整串查命中「查明」。
//
// 不变量（单测 `first_agrees_with_full_lookup` 守着）：`first(c)` 必须等于
// `lookup(c, 上限)[0]`。**任何调整候选上限的改动都不许影响这里的结果。**

/// 有没有候选（只问存在性，不关心是谁）。
pub fn any(prefix: &str) -> bool {
    search(false, prefix, "", AssistMode::Off, 1).first().is_some()
}

/// 带辅助码筛选后还有没有候选。
pub fn any_filtered(prefix: &str, assist_prefix: &str, mode: AssistMode) -> bool {
    search(false, prefix, assist_prefix, mode, 1)
        .first()
        .is_some()
}

/// 全拼：有没有候选。
pub fn any_pinyin(prefix: &str) -> bool {
    search(true, prefix, "", AssistMode::Off, 1)
        .first()
        .is_some()
}

/// 全拼 + 辅助码：有没有候选。
pub fn any_pinyin_filtered(prefix: &str, assist_prefix: &str, mode: AssistMode) -> bool {
    search(true, prefix, assist_prefix, mode, 1)
        .first()
        .is_some()
}

/// 双拼：第一名是什么（跑完整流水线，和候选窗第一条**必须**一致）。
pub fn first(prefix: &str) -> Option<Cow<'static, str>> {
    search(false, prefix, "", AssistMode::Off, usize::MAX)
        .into_iter()
        .next()
}

/// 双拼 + 辅助码筛选：第一名是什么。
pub fn first_filtered(
    prefix: &str,
    assist_prefix: &str,
    mode: AssistMode,
) -> Option<Cow<'static, str>> {
    search(false, prefix, assist_prefix, mode, usize::MAX)
        .into_iter()
        .next()
}

/// 全拼：第一名是什么。
pub fn first_pinyin(prefix: &str) -> Option<Cow<'static, str>> {
    search(true, prefix, "", AssistMode::Off, usize::MAX)
        .into_iter()
        .next()
}

/// 全拼 + 辅助码筛选：第一名是什么。
pub fn first_pinyin_filtered(
    prefix: &str,
    assist_prefix: &str,
    mode: AssistMode,
) -> Option<Cow<'static, str>> {
    search(true, prefix, assist_prefix, mode, usize::MAX)
        .into_iter()
        .next()
}

fn search(
    pinyin: bool,
    prefix: &str,
    assist_prefix: &str,
    mode: AssistMode,
    limit: usize,
) -> Vec<Cow<'static, str>> {
    // **模糊音：先把输入展开成几个「同音异写」的码，逐个查询再合并。**
    //
    // 展开结果的第一项恒等于原样，所以原样查询就是 `collect(prefix)`；后面每一项
    // 都算「模糊命中」，在排序里要让一步（见 `freq::rank` 的 `tiers`）。
    //
    // 关掉模糊音时**一次都不展开**（`Vec::new()` 不分配）：`search` 每次按键都跑，
    // 为了一个默认关闭的功能多一次 `String` 分配不划算。
    //
    // 每个变体只取前 `FUZZY_PER_VARIANT` 条：模糊音是「原样不够用时的补救」，
    // 不是把整库同音字翻出来；不设上限的话，单键查询要多扫好几遍全表。
    let cfg = crate::config::get();
    let alts: Vec<String> = if pinyin || !cfg.fuzzy.any() {
        Vec::new()
    } else {
        crate::fuzzy::expand(cfg.fuzzy, cfg.scheme, prefix)
            .into_iter()
            .skip(1) // 第 0 项就是原样
            .collect()
    };
    let (mut out, mut lens) = collect(pinyin, prefix, assist_prefix, mode, limit);
    let mut tiers: Vec<u8> = vec![0; out.len()];

    if !alts.is_empty() {
        let sub = limit.min(FUZZY_PER_VARIANT);
        for code in &alts {
            let (o, l) = collect(pinyin, code, assist_prefix, mode, sub);
            for (w, len) in o.into_iter().zip(l) {
                out.push(w);
                lens.push(len);
                tiers.push(1);
            }
        }
        // 同一个词可能既被原样命中也从某个模糊变体冒出来（`南` 的码是 `nj`，
        // 但打 `lj` 时它也会从 `lj` 那一支出现）。去重保留**先出现的**——
        // 原样命中排在前面，所以先出现的自然就是精确那一份。
        dedupe_keep_first(&mut out, &mut lens, &mut tiers);
    }

    // 自造词也进候选，**和词库候选一起交给下面那次词频排序**。
    //
    // 以前是「插到 0 号位」：刚拼出来的 `limkyr` 下次就该直接出。但自造词本身也是
    // 学来的——`commit_segments` 每分词上屏一次就记一次，用户并没有「造词」的意思。
    // 无条件置顶于是变成「谁最后被分词提交过，谁永远第一」：反复从候选里选「自然码」，
    // 第一位却一直是早前分词学出来的「自然吗」，动态词频对它完全无效（用户报的
    // 就是这一条）。现在两者用同一把尺子——`freq` 的「码 + 词」次数，谁被选得多谁在
    // 前。刚造的词由 `remember_phrase` 顺手记一次次数，而对手是 0 次，所以「刚造的
    // 立刻能出」这个原始需求照样成立。
    //
    // 码长按「等于输入」算：自造词只认整码（`phrase::words_with_code`），它的码就是
    // 输入本身，也就是这一批候选里最短的那一档，和词库里打满的那个词平级。
    if assist_prefix.is_empty() {
        for w in crate::phrase::words_for(prefix, pinyin) {
            if out.iter().any(|x| x.as_ref() == w.as_str()) {
                continue;
            }
            lens.push(prefix.len().min(u8::MAX as usize) as u8);
            out.push(Cow::Owned(w));
        }
    }

    // 动态词频只在这一层做：双拼/全拼、带不带辅助码的查询全都汇到这里，
    // 不会出现「某个入口忘了排序」这种只在特定模式下复现的怪事。
    //
    // 注意：这里**不再有**「先截断再排序」这一步。早期候选有 90 条上限时，
    // 学习记录只能把「本来就在前 90 名里」的词往前挪，排在外面的冷僻词永远
    // 捞不进来；现在候选不设上限，整段都参与排序，学过的词再冷门也能浮上来。
    // 学到的次数按「码 + 词」记：多音字同一个词挂多条读音，只按词记会让在
    // `oo` 里选过的「哦」跑到 `ee` 的第一位，顶掉饿 / 呃 / 额。
    crate::freq::rank(&mut out, &lens, &tiers, prefix);

    // 手动降权：被按下去的词沉到最后，**只重排、不删除**。
    //
    // 位置有讲究：紧跟在词频排序之后，所以后面每一道「显式提权」（一简字 /
    // 二简词 / 自定义短语 / 置顶）都还能把它拽回前面——降权压的是「默认排序」，
    // 不是用户的显式选择。反过来说，放在 `freq::rank` 之前会白做：排序立刻又
    // 把它顶上来。
    crate::demote::push_back(&mut out, prefix);

    // 查询只覆盖一个音节？
    //
    // 双拼每个音节恰好两键，所以「两个字符」就是「一个音节」；全拼长度不定，
    // 复用 `is_syllable`（和智能分隔用的是同一个判定，不会出现两套标准）。
    let one_syllable = if pinyin {
        crate::shuangpin::is_syllable(prefix)
    } else {
        prefix.chars().count() == 2
    };

    // 完整辅助码：形码打满（自然码 1 键 / 小鹤 2 键）且**查询只覆盖一个音节**时，
    // 首字形码与输入**完全相等**的单字就是用户要的那个字（`jm` + `vo` → 简），
    // 提到最前。
    //
    // 为什么必须做：筛选只按「首字形码是输入的前缀」算，单字和词组一视同仁，
    // 排序只看词频——「简单」比「简」常用，形码打满后第一候选还是「简单」，
    // 用户要的精确单字沉在后面，3 键直出 + 完整形码等于白打。形码打满意味着
    // 用户已经把首字「指定到字」了，此时精确命中的单字理应赢过仅前缀命中的词组。
    //
    // 为什么还要「一个音节」这个前提：自然码的辅助码只有 1 键，`max_keys()` 就是 1，
    // 于是**敲第一个辅助码字母就算打满**。而两字词也支持按首字筛辅助码
    // （`nihc` + `r` 找「你好」），不设这道闸的话，你（部首 亻）会被提到
    // 「你好」前面——用户明明打了两个音节，出来的却是单字。
    // 单音节查询才是「我在指定这个字」，多音节查询是「我在筛这个词」。
    promote_exact_shape(&mut out, assist_prefix, mode, one_syllable);

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

    // 自定义短语：用户手写的「码 → 文本」，直接顶到候选最前面。
    //
    // 为什么不和词库候选一起交给 `freq::rank`：这是**配置**，不是学习结果。用户写了
    // `dz` → 地址，打 `dz` 第一位就该是那个地址，不该先被「上次从词库里选了别的」
    // 顶几次才慢慢学会。同一个码写多行时按文件顺序依次占 0、1、2 位。
    //
    // 带辅助码筛选时不参与（同自造词、二简词）：用户已经主动筛了一道形，
    // 而短语是一整串文本，没有「首字形码」能对上。
    //
    // 放在置顶**之前**：置顶是「临时想换一个」的即时动作，比改文件更近，
    // 所以让它压在自定义短语上面。
    if assist_prefix.is_empty() {
        for (i, w) in crate::custom::words_for(prefix).iter().enumerate() {
            place(&mut out, w, i, limit);
        }
    }

    // 手动置顶压在最后，也就是排在最前：用户显式钉的东西必须赢过一切默认值。
    // 这里 `insert_missing = true`——置顶的全部意义就是「哪怕它不在候选里也要出」。
    if let Some(w) = crate::pin::word_for(prefix) {
        promote(&mut out, &w, true, limit);
    }
    // 自造词是排序前插进去的，不经过 `promote` 的截断，这里统一收口。
    out.truncate(limit);
    out
}

/// 完整辅助码下，把「首字形码与输入完全相等」的单字稳定地提到候选最前。
///
/// 稳定 = 提上去的若干单字保持原有相对顺序（同为精确命中时词频高的还在前），
/// 其余候选原顺序跟在后面。只重排、不插入：形码相等的字不在候选里就说明它
/// 被读音前缀筛掉了，硬塞进来反而是错字。
/// 抽成独立函数是为了单测（`search` 依赖 mmap 的词库，测试进程构造不自由）。
///
/// `one_syllable` 见调用点的注释：多音节查询是「筛词」，不该把首字提到词前面。
fn promote_exact_shape(
    out: &mut Vec<Cow<'static, str>>,
    assist_prefix: &str,
    mode: AssistMode,
    one_syllable: bool,
) {
    if mode == AssistMode::Off || assist_prefix.is_empty() || !one_syllable {
        return;
    }
    // 没打满 = 还是前缀筛选语义，不动排序
    if assist_prefix.chars().count() != mode.max_keys() {
        return;
    }
    let exact = |w: &str| {
        w.chars().count() == 1
            && crate::assist::code_of(w.chars().next().unwrap(), mode)
                .is_some_and(|c| c.split('|').any(|one| one == assist_prefix))
    };
    let mut head = Vec::new();
    let mut rest = Vec::with_capacity(out.len());
    for w in out.drain(..) {
        if exact(&w) {
            head.push(w);
        } else {
            rest.push(w);
        }
    }
    out.reserve(head.len() + rest.len());
    out.extend(head);
    out.extend(rest);
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

/// 反查一批**单字**的读音（全拼，音节用空格分隔）。
///
/// 只认单字条目：`璇玑` 这条词条只说明整词的拼音，单个字的读音得从它自己的
/// 单字条目里拿。多音字（`重` chong / zhong）会返回多条，顺序就是索引顺序
/// （按码），调用方自己决定取哪条。
///
/// 为什么不缓存：一次 `/fc` 只扫一遍（几万条内存映射，约 1ms），而缓存要在
/// 用户导入词库、切换方案后失效——为一个低频命令维护一套失效逻辑不划算。
pub fn readings_for(chars: &[char]) -> Vec<Vec<String>> {
    use std::collections::HashMap;

    let mut out: Vec<Vec<String>> = vec![Vec::new(); chars.len()];
    if chars.is_empty() {
        return out;
    }
    let mut want: HashMap<char, Vec<usize>> = HashMap::new();
    for (i, &c) in chars.iter().enumerate() {
        want.entry(c).or_default().push(i);
    }

    fn take(out: &mut [Vec<String>], want: &HashMap<char, Vec<usize>>, w: &str, py: &str) {
        // 单字最多 4 字节（汉字 3 字节、扩展区 4 字节），更长的必然是词
        if w.len() > 4 {
            return;
        }
        let mut it = w.chars();
        let Some(c) = it.next() else { return };
        if it.next().is_some() {
            return;
        }
        let Some(idx) = want.get(&c) else { return };
        for &i in idx {
            if !out[i].iter().any(|s| s == py) {
                out[i].push(py.to_string());
            }
        }
    }

    let user = user_source();
    match &user {
        UserSrc::Mapped(b) => {
            for i in 0..b.count() {
                take(&mut out, &want, b.word(i), b.pinyin(i));
            }
        }
        UserSrc::Heap(ov) => {
            for (w, py) in &ov.entries {
                take(&mut out, &want, w, py);
            }
        }
        UserSrc::None => {}
    }

    let bin = dictbin::get();
    for i in 0..bin.count() {
        take(&mut out, &want, bin.word(i), bin.pinyin(i));
    }
    out
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
    // 额度不小于候选数（含 `usize::MAX` = 不设上限）时没有「挑前 K 个」可言，
    // 直接全排序：O(n log n) 一次搞定。走下面的插入式挑选会退化成 O(n × limit)——
    // 单键（`n` 命中几千条）传 MAX 就等于几千次二分插入 + 搬移，按键当场卡住。
    //
    // `sort_by_key` 是稳定排序，同键保持词库原顺序，和插入式挑选的结果一致。
    if range.len() <= limit {
        let mut all: Vec<((usize, usize), usize)> = range.map(|p| (key_of(p), p)).collect();
        all.sort_by_key(|x| x.0);
        return all.into_iter().map(|(_, p)| p).collect();
    }

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

/// 长词收窄的**尺度**：词码长度最多比输入长一个音节。
///
/// 双拼一个音节 2 键，全拼取 4 个字母（`yu lan` 这类最常见的情况）。
/// 抽成函数是为了能单测这条边界——它一旦算错，症状是「候选太多，翻不到想要的」
/// 或者「长词永远不出现」，两种都不像代码错误。
fn reach_cap(prefix_len: usize, pinyin: bool) -> usize {
    prefix_len + if pinyin { 4 } else { 2 }
}

/// 往候选序列里放一条。
///
/// `sorted` 为真时按**码长**插入（同码长排在已有条目之后，所以用户词库天然在
/// 核心词库前面）。这个函数存在的原因见 `collect` 里「带辅助码时两个词库都要收」
/// 那段：两段各自已按 (码长, 行号) 有序，插进同一个序列就是归并，不用全排序。
fn put_candidate(
    out: &mut Vec<Cow<'static, str>>,
    lens: &mut Vec<u8>,
    w: Cow<'static, str>,
    len: u8,
    limit: usize,
    sorted: bool,
) {
    if !sorted {
        out.push(w);
        lens.push(len);
        return;
    }
    let pos = lens.iter().position(|&x| x > len).unwrap_or(lens.len());
    // 已经满额且比现有最长的还长：这条排不进候选，直接丢
    if pos >= limit {
        return;
    }
    out.insert(pos, w);
    lens.insert(pos, len);
    if out.len() > limit {
        out.pop();
        lens.pop();
    }
}

/// 模糊音每个变体最多取几条候选。
///
/// 模糊音是「原样查不到时给个台阶」，不是把整库同音字翻出来。不设上限的话，
/// 单键查询（`a`、`n` 这种）要多扫好几遍全表，而用户根本看不到第 97 条。
const FUZZY_PER_VARIANT: usize = 96;

/// 按词去重，保留**先出现的**那一份，三个平行数组一起删。
///
/// 为什么不用 `HashSet<&str>` + `retain`：`retain` 拿着 `&mut`，而集合借着 `out`
/// 里的字符串，借用检查过不去。折成两步——先标记，集合销毁后再删。
fn dedupe_keep_first(
    out: &mut Vec<Cow<'static, str>>,
    lens: &mut Vec<u8>,
    tiers: &mut Vec<u8>,
) {
    debug_assert_eq!(out.len(), lens.len());
    debug_assert_eq!(out.len(), tiers.len());
    let mut drop = vec![false; out.len()];
    {
        let mut seen: std::collections::HashSet<&str> =
            std::collections::HashSet::with_capacity(out.len());
        for (i, w) in out.iter().enumerate() {
            if !seen.insert(w.as_ref()) {
                drop[i] = true;
            }
        }
    }
    let mut i = 0;
    out.retain(|_| {
        let keep = !drop[i];
        i += 1;
        keep
    });
    let mut i = 0;
    lens.retain(|_| {
        let keep = !drop[i];
        i += 1;
        keep
    });
    let mut i = 0;
    tiers.retain(|_| {
        let keep = !drop[i];
        i += 1;
        keep
    });
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
    // 去重集合。以前是 `Vec` + 线性 `contains`，候选少时确实更快；但候选不设上限
    // 之后单键能到上千条，那就是 O(n²)（`n` 键 1000 条 ≈ 2 ms，按键能感觉到）。
    // `HashSet` 一次插入替代「扫一遍 + push」，上千条降到几百微秒。
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();

    let pass = |w: &str| -> bool { assist::word_matches(w, mode, assist_prefix) };

    // **两个词库都要收，按码长归并——「用户词库收满就直接返回」是错的。**
    //
    // 老逻辑是「用户词库优先，收满 limit 条就不再扫核心词库」。不带辅助码时
    // 看着没问题（limit 不设上限），但它有一个致命副作用：**个人词库会把核心
    // 词库整体挤出候选**。用户导入搜狗词表（5.5 万条）之后，`ia`（cha）的前几
    // 百条候选全是「查看 / 查询 / 查的」这类两字词，核心词库的单字「查」一条
    // 都进不来。
    //
    // 症状有两条：
    //   1. 打 `ia` 找不到「查」这个字（翻页也翻不到，候选是被过滤掉的）；
    //   2. 3 键直出的判定是「筛出的第一项必须是单字」。`ia` + 自然码 `m`（查→木）
    //      的第一项是词「查看」，于是拒绝拆分，`iam` 被当成整串码去查，正好命中
    //      「查明」——用户看到的就是「自然码一个辅助码打满却精确不出字」。
    //
    // 码长在这里就是仲裁者：单字 2 键、两字词 4 键，`top_by_rank` 已按
    // 「码长 → 行号」排过，所以两段各自有序，把核心词库的条目按码长插进来即可，
    // 不必全排序。同码长时先来的（用户词库）在前，保持「个人词优先」的老语义。
    //
    // **只在带辅助码时归并。** 不带辅助码时 `limit` 是「候选不设上限」的
    // `usize::MAX`，用户词库永远填不满、核心词库照样会被扫到，没有必要为它
    // 按码长插队——而且那时候 `out` 有几千条，逐条 `insert` 是 O(n²)。
    let filtered = !assist_prefix.is_empty();


    // **带辅助码时必须先扫全量、再截断。**
    //
    // `top_by_rank` 是「按词频挑前 limit 条」，过滤发生在它之后。limit 很小时
    // （判定命中那一路传的是 1），它只挑出该码下词频最高的那一条：`ji` 的第一名
    // 是「机」，辅助码 `wj` 命中不了它，于是 limit=1 的筛选结果恒为空。
    // 症状就是「3 键直出永远不拆」和「辅助码对这个字无效」——用户按反查得到的
    // `jiwj` 敲下去一个字都出不来。
    //
    // 不改 `top_by_rank` 去接受过滤闭包：那条热路径上没有辅助码时不能背这个
    // 开销（单键几千条）。只在对辅助码筛选时把额度放开，让 `top_by_rank` 走
    // 全排序分支，过滤后再由下面的 `out.len() >= limit` 提前收工。
    let scan = if assist_prefix.is_empty() { limit } else { usize::MAX };

    // **长词收窄：词的码长最多比输入多「一个音节」。**
    //
    // 前缀匹配的方向是「输入串是词码的前缀」，这是增量输入必需的：打 `wfjmyulj`
    // 的过程中，前 6 键必须能看见「文件预览」，否则这个码根本打不出来。但前缀匹配
    // 本身没有远近概念——任何更长的词只要码以它开头就合格，于是两个音节的输入
    // （`wfjm` = wen jian）会把个人词库里所有「文件X」的短语全捞出来：实测 178 条，
    // 3 字词、4 字词一路排下去，用户要的「文件」只在第一条。
    //
    // 收窄尺度取「一个音节」：双拼 2 键、全拼 4 字母。它刚好保住「快到手的长词现在
    // 就能看见」（`wfjmyu` 时出现「文件预览」），又不让两个音节的输入被四字词淹掉。
    // 比这更严（必须打满才出现）会在打长词的中途出现空候选窗——`wfjmyul`（7 键）
    // 时既没有 7 键的词、长词又被藏了，用户会以为输入法卡死。
    //
    // 迭代顺序是 `(码长, 行号)` 升序，所以一旦超出就可以直接 `break`，不用逐条比。
    let cap = reach_cap(prefix.len(), pinyin);

    // 1) 用户词库优先。常态是 mmap 的 user.bin（和 base.bin 同一条零解析路径，
    //    多进程共享页缓存）；只有 bin 缺失或比 txt 旧时才退回堆索引。
    //
    // `user` 必须在 match 之外绑定：`seen` 借用它的字符串，作用域要比 match 长。
    let user = user_source();
    match &user {
        UserSrc::Mapped(b) => {
            let (ps, pe) = b.prefix_range(which, prefix);
            for p in top_by_rank(ps..pe, scan, |p| b.rank_key(which, p)) {
                let code_len = b.rank_key(which, p).0;
                if code_len > cap {
                    break; // 迭代按码长升序，后面只会更长
                }
                let w = b.word_at(which, p);
                if !pass(w) || !seen.insert(w) {
                    continue;
                }
                put_candidate(
                    &mut out,
                    &mut lens,
                    Cow::Borrowed(w),
                    code_len.min(u8::MAX as usize) as u8,
                    limit,
                    filtered, // 带辅助码：统一按码长插；否则用户词库内部已有序，直接追加
                );
                if out.len() >= limit {
                    if !filtered {
                        return (out, lens);
                    }
                    break; // 额度满了，剩下的位置留给核心词库
                }
            }
        }
        UserSrc::Heap(ov) => {
            let (ps, pe) = ov.prefix_range(which, prefix);
            for p in top_by_rank(ps..pe, scan, |p| ov.rank_key(which, p)) {
                let code_len = ov.rank_key(which, p).0;
                if code_len > cap {
                    break;
                }
                let w = ov.word_at(which, p);
                if !pass(w) || !seen.insert(w) {
                    continue;
                }
                put_candidate(
                    &mut out,
                    &mut lens,
                    Cow::Owned(w.to_string()),
                    code_len.min(u8::MAX as usize) as u8,
                    limit,
                    filtered,
                );
                if out.len() >= limit {
                    if !filtered {
                        return (out, lens);
                    }
                    break;
                }
            }
        }
        UserSrc::None => {}
    }

    // 2) 静态词库
    let bin = dictbin::get();
    {
        let (ps, pe) = bin.prefix_range(which, prefix);
        // 归并时核心词库自己收满 limit 条就够：`top_by_rank` 已经按码长排过，
        // 前 limit 条命中就是码长最短的 limit 条，后面的插不进候选。
        let mut taken = 0usize;
        for p in top_by_rank(ps..pe, scan, |p| bin.rank_key(which, p)) {
            let code_len = bin.rank_key(which, p).0;
            if code_len > cap {
                break;
            }
            let w = bin.word_at(which, p);
            if !pass(w) || !seen.insert(w) {
                continue;
            }
            put_candidate(
                &mut out,
                &mut lens,
                Cow::Borrowed(w),
                code_len.min(u8::MAX as usize) as u8,
                limit,
                filtered, // 带辅助码：按码长插进用户词库那一段里
            );
            taken += 1;
            if filtered && taken >= limit {
                break;
            }
            if !filtered && out.len() >= limit {
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

/// 用户词库当前**实际生效**的来源（设置窗展示用）。
///
/// 「user.bin 存在」不等于「在用」：`user_dict.txt` 比 bin 新（手改了没重新编译）
/// 或 bin 读不出来时，运行时退回文本。把这个状态亮出来，用户就不用猜
/// 「编译出来的 user.bin 到底有没有被用」。
pub fn user_source_label() -> &'static str {
    match user_source() {
        UserSrc::Mapped(_) => "user.bin 已启用",
        UserSrc::Heap(_) => "user_dict.txt 文本回退（重新编译后启用 user.bin）",
        UserSrc::None => "未导入个人词库",
    }
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
    merge_text(&text)
}

/// 把一份「`词<TAB>全拼`」文本并进个人词库，然后立刻编译成 `user.bin`。
///
/// 和 `import_from` 的区别只是「文本从哪来」：一个是用户选的文件，一个是
/// 搜狗词库解析出来的内存串（`sogou` 模块）。**合并规则只能有一份**——
/// 两个入口各自实现一遍去重，迟早会变成「文件导入去重、搜狗导入不去重」。
///
/// 返回 `(新增条目数, Some((bin 条目数, bin 字节数)))`；第二个为 `None`
/// 表示编译失败、退回文本覆盖层（导入本身是成功的）。
pub fn merge_text(text: &str) -> Result<(usize, Option<(usize, usize)>), String> {
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
        // 原子写：直接 `write` 会在崩溃/断电时留下半截文件，而这是用户几年的词库
        config::write_atomic(&user_file(), &all).map_err(|e| format!("写入失败：{e}"))?;

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

/// `SCHEME` / `pinyin_mode` 都是进程级全局状态，测试并行跑会互相篡改。
/// 凡是碰这两个状态的用例（**包括壳那个 crate 里的**）统一拿这把锁串行。
///
/// 和 `config::test_guard` 同一个原因不能加 `#[cfg(test)]`：`dict` 搬进 core 之后，
/// 调用方有一半在 `feihe` 那个 crate 里，core 正常编译时这个函数不存在就编不过。
#[doc(hidden)]
pub fn scheme_guard() -> std::sync::MutexGuard<'static, ()> {
    static L: std::sync::Mutex<()> = std::sync::Mutex::new(());
    L.lock().unwrap_or_else(|e| e.into_inner())
}

/// 词库**源文件**所在目录：仓库根的 `src/`。
///
/// `dict` 已经搬进 `feihe-core`，但 `dict.txt` / `extra_*.txt` / `user_dict.txt`
/// 仍由 `tools/gen_dict.py` / `gen_extra.py` 写在仓库根的 `src/` 下（见 `core/build.rs`
/// 的同一条说明）。测试里凡是要读这些文件的，都从这里拼路径——
/// **不要用相对路径 `"src"`**：`cargo test` 的 CWD 是 crate 根（也就是 `core/`），
/// 那样会读到 `core/src`，一个词库文件都没有，测试会「因为读不到文件而空跑变绿」。
#[cfg(test)]
fn repo_src() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex;




    /// **自造词不再无条件置顶：它和词库候选一起按学习次数排。**
    ///
    /// 现场：用户的 `zirjma` 下有个早前分词时自动学出来的「自然吗」（自造词），
    /// 无论他从候选里选多少次「自然码」，第一位永远是「自然吗」——动态词频对它
    /// 完全无效，看起来就像「词频调整坏了」。
    ///
    /// 自造词本身也是学来的（`commit_segments` 每分词上屏一次就记一次），凭什么
    /// 压过用户显式选择的词？现在的规则是同一把尺子：谁在这个码下被选得多谁在前。
    ///
    /// 词面从**词库自己**取，不写死：单测不能假设词库里恰好有哪个词。
    #[test]
    fn learned_candidate_beats_auto_phrase() {
        let _g = scheme_guard();
        set_scheme(Scheme::Xiaohe);
        // `zirj`（zi ran）：词库里自然 / 孜然 都是两字，码长一样，只差词频——正好
        // 是这次要比的那一对。词面从**词库自己**取，不写死。
        let base = lookup("zirj", usize::MAX);
        let head_len = base.first().expect("该码在词库里必须有候选").chars().count();
        let target = base
            .iter()
            .skip(1)
            .find(|w| w.chars().count() == head_len)
            .expect("需要一个同码长的候选来对比")
            .to_string();

        // 同一个码下挂一个自造词：老逻辑会把它插到 0 号位
        crate::phrase::learn("字然", "zi ran");
        // 用户反复从候选里选了 target
        for _ in 0..3 {
            crate::freq::bump("zirj", &target);
        }

        let r = lookup("zirj", usize::MAX);
        assert_eq!(
            r.first().map(|s| s.as_ref()),
            Some(target.as_str()),
            "选过三次的词必须赢过自动学的自造词：{r:?}"
        );
    }

    /// 自造词仍然要出得来——它只是不再插队，不是不出现。
    #[test]
    fn auto_phrase_still_listed_after_ranking() {
        let _g = scheme_guard();
        set_scheme(Scheme::Xiaohe);
        crate::phrase::learn("字然", "zi ran");
        let r = lookup("zirj", usize::MAX);
        assert!(r.iter().any(|w| w.as_ref() == "字然"), "自造词不能消失：{r:?}");
    }

    /// 长词收窄的边界：最多比输入长一个音节。
    ///
    /// 现场：`wfjm`（wen jian，4 键）返回 178 条——个人词库里所有「文件X」的短语
    /// 都因为「码以 wfjm 开头」被捞出来，3 字词、4 字词一路排下去，用户要的
    /// 「文件」只在第一条，翻页翻半天。
    #[test]
    fn reach_allows_one_extra_syllable() {
        // 双拼：一个音节 2 键
        assert_eq!(reach_cap(4, false), 6, "两音节输入：最多看到三字词");
        assert_eq!(reach_cap(1, false), 3, "单键：只看到单字");
        assert_eq!(reach_cap(6, false), 8, "三音节输入：四字词该出现了");
        assert_eq!(reach_cap(8, false), 10, "码打满：自己一定能出");
        // 全拼：一个音节按 4 个字母算
        assert_eq!(reach_cap(7, true), 11);
        assert_eq!(reach_cap(12, true), 16);
    }

    /// 两音节输入里不该出现四字以上的词（`wfjm` 不该带出「文件预览」）。
    ///
    /// 只断言上界，不假设词库里有哪条长词——那条规矩踩过两次。
    #[test]
    fn two_syllable_input_has_no_long_words() {
        let _g = scheme_guard();
        set_scheme(Scheme::Xiaohe);
        for code in ["wfjm", "ddj", "xrji", "nihc"] {
            let list = lookup(code, usize::MAX);
            assert!(
                list.iter().all(|w| w.chars().count() <= 3),
                "`{code}`（{} 键）不该有两字以上的词：{:?}",
                code.len(),
                list.iter().filter(|w| w.chars().count() > 3).take(5).collect::<Vec<_>>()
            );
        }
    }

    /// **探测入口与候选上限无关**：`first*` 拿到的必须是 `lookup(..., 上限)` 的第一条。
    ///
    /// 这是「3 键直出 / 自动并回 / 分词默认字」这一整片逻辑的地基——它们全都只问
    /// 「第一名是谁」，一旦某次改动让探测和显示两条路的头部不一致，判定就会失效，
    /// 而症状永远不会指向词库（`iam` 那次的表现是「精确不出字」）。
    ///
    /// 这条用例只比「自己和自己」，不假设词库里有什么，所以不会被个人词库影响；
    /// 反过来说它也**不能**替换成具体词断言（那条规矩踩过两次）。
    #[test]
    fn first_agrees_with_full_lookup() {
        let _g = scheme_guard();
        set_scheme(Scheme::Xiaohe);

        for code in ["ji", "dd", "ia", "n", "zuci", "fwhe"] {
            let full = lookup(code, usize::MAX);
            assert_eq!(
                first(code).as_deref(),
                full.first().map(|w| w.as_ref()),
                "`{code}`：探测拿到的第一名必须和全量查询一致"
            );
        }

        // 带辅助码那条路：首名同样要和全量筛选一致
        for mode in [AssistMode::Ziranma, AssistMode::Xiaohe] {
            for (code, assist) in [("ia", "m"), ("dd", "j"), ("jm", "vo"), ("ji", "wj")] {
                let full = lookup_filtered(code, assist, mode, usize::MAX);
                assert_eq!(
                    first_filtered(code, assist, mode).as_deref(),
                    full.first().map(|w| w.as_ref()),
                    "`{code}`+`{assist}`（{mode:?}）：辅助码路径的探测也必须和全量一致"
                );
            }
        }
    }

    /// 带辅助码时，核心词库的短码单字不能被个人词库挤出候选。
    ///
    /// 用户报的现场：导入搜狗个人词表后打 `ia` + 自然码 `m`（查→木），候选全是
    /// 「查看 / 查询 / 查的」这类两字词，单字「查」连一屏都进不去。3 键直出的
    /// 「第一项必须是单字」判定因此不成立、拒绝拆分，`iam` 被当成整串码查，
    /// 正好命中「查明」——用户看到的就是「自然码一个辅助码打满却精确不出字」。
    ///
    /// 这里只断言归并语义，不碰真实词库内容（个人词库随用户导什么而变）。
    #[test]
    fn merged_candidates_keep_short_code_from_static_dict() {
        let mut out: Vec<Cow<'static, str>> = Vec::new();
        let mut lens: Vec<u8> = Vec::new();
        // 个人词库：三条 4 键两字词把 3 个额度占满
        for w in ["查看", "查询", "查的"] {
            put_candidate(&mut out, &mut lens, Cow::Borrowed(w), 4, 3, true);
        }
        // 核心词库的 2 键单字仍要挤进来，把最长的那条顶掉
        put_candidate(&mut out, &mut lens, Cow::Borrowed("查"), 2, 3, true);
        assert_eq!(
            out.iter().map(|w| w.as_ref()).collect::<Vec<_>>(),
            ["查", "查看", "查询"],
            "短码单字必须挤回候选，额度满也要顶掉最长的一条"
        );
        // 同码长时先来的（个人词库）在前：个人词优先的老语义不能丢
        put_candidate(&mut out, &mut lens, Cow::Borrowed("察"), 2, 4, true);
        assert_eq!(
            out.iter().map(|w| w.as_ref()).collect::<Vec<_>>(),
            ["查", "察", "查看", "查询"]
        );
    }



    /// 生成词表的每条都必须**真的能查到**。
    ///
    /// 词库存的是全拼，拼音错了码就错了，词等于不存在——而这批词是 pypinyin 算的，
    /// 错一个不会有任何提示。所以这里把 6 份 `src/extra_*.txt` 全量跑一遍：
    /// 按文件里的拼音编码，再按码查，必须能查回这个词本身。
    ///
    /// 挂了就去 `tools/pinyin_overrides.txt` 钉读音，然后重跑 `tools/gen_extra.py --write`。
    /// 别改这个测试去迁就数据。
    #[test]
    fn generated_words_are_all_searchable() {
        let _g = scheme_guard();
        set_scheme(Scheme::Xiaohe);
        let mut bad = Vec::new();
        let mut total = 0;
        // 扫目录而不是写死清单：`tools/gen_extra.py` 加一个分类，这里要能自动跟上，
        // 否则新分类的词「生成出来了但没人验」，拼音错了也没人发现。
        let mut files: Vec<_> = std::fs::read_dir(repo_src())
            .expect("读不到 src/")
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.starts_with("extra_") && n != "extra_dict.txt" && n.ends_with(".txt"))
            .collect();
        files.sort();
        assert!(files.len() >= 10, "分类词表只找到 {} 份", files.len());
        for name in files {
            let text = std::fs::read_to_string(repo_src().join(&name)).unwrap_or_default();
            for line in text.lines() {
                let Some((w, py)) = line.split_once('\t') else { continue };
                let Some(code) = shuangpin::encode_pinyin(Scheme::Xiaohe, py) else {
                    bad.push(format!("{w} 编码失败 {py}"));
                    continue;
                };
                total += 1;
                // 不设上限：`lookup(code, 90)` 会让「同码条目排到 90 名之后」被误判成
                // 「查不到」。用户报的「玑」就是这么被藏起来的——候选额度一砍，
                // 测试也跟着学坏，两边一起把字弄丢。
                let r = lookup(&code, usize::MAX);
                if !r.iter().any(|x| &**x == w) {
                    bad.push(format!("{w} 查不到 code={code} py={py}"));
                }
            }
        }
        assert!(total > 5000, "词表没读进来？只检查了 {total} 条");
        assert!(
            bad.is_empty(),
            "{} 条查不到（前 20 条：{:?}）",
            bad.len(),
            &bad[..bad.len().min(20)]
        );
    }

    #[test]
    fn dict_not_empty() {
        let (total, _, _) = stats();
        assert!(total > 1000, "dict too small: {total}");
    }

    /// 全量审计：`dict.txt` 里每一条，按它自己的拼音编出小鹤码后都得能查回来。
    ///
    /// 上面的 `generated_words_are_all_searchable` 只覆盖 `extra_*.txt`（自动生成的
    /// 分类词表），主词库 4.6 万条一直没人这样验。这条补上，并且**不设候选上限**——
    /// 「同码条目多、排到后面」不算查不到，那种情况下用户翻页能翻到。
    ///
    /// 跑法：`cargo test --lib audit_dict_entries_reachable -- --ignored --nocapture`
    /// 人工看纠错在真实词库下的表现。
    ///
    /// 跑法：`cargo test --lib zz_probe_correction -- --ignored --nocapture`
    #[test]
    #[ignore = "人工看输出用，不做断言（依赖真实词库内容）"]
    fn zz_probe_correction() {
        let _g = scheme_guard();
        set_scheme(Scheme::Xiaohe);
        // 左列是「按错了一个键」的码，右列是期望救回来的词
        let cases = [
            ("nihx", "你好"),  // c(ao) 按成 x(ua)
            ("nihd", "你好"),  // 其实合法（hai → 你还），不该纠
            ("vsgp", "中国"),  // o(uo) 按成 p(ie)
            ("vshx", "中国"),  // 第二音节整个错
            ("dihc", "电话"),  // 第一个音节错
            ("nihxma", "你好吗"),
            ("nihcma", "你好吗"), // 有词，不该纠
        ];
        for (code, want) in cases {
            let direct = search(false, code, "", AssistMode::Off, 3);
            let d: Vec<&str> = direct.iter().map(|w| w.as_ref()).collect();
            let got = corrected(code, false, 8);
            let words: Vec<&str> = got.iter().map(|w| w.as_ref()).collect();
            let hit = words.contains(&want);
            println!("{code:<8} 直接={d:?}  纠错命中{hit}={words:?}");
        }
        // 一次纠错要查多少次词库、花多久：这是热路径上的额外开销，得有个数
        for code in ["nihx", "vsgp", "nihxma"] {
            let n = corrected(code, false, 8).len();
            let t = std::time::Instant::now();
            for _ in 0..200 {
                let _ = corrected(code, false, 8);
            }
            println!(
                "{code:<8} 候选 {n} 条，单次 {:?}",
                t.elapsed() / 200
            );
        }
    }

    #[test]
    #[ignore = "全量审计（4.6 万条），平时不跑"]
    fn audit_dict_entries_reachable() {
        let _g = scheme_guard();
        set_scheme(Scheme::Xiaohe);
        let text = std::fs::read_to_string(repo_src().join("dict.txt")).unwrap_or_default();
        let mut bad: Vec<String> = Vec::new();
        let mut total = 0usize;
        for line in text.lines() {
            let Some((w, py)) = line.split_once('\t') else {
                continue;
            };
            let (w, py) = (w.trim(), py.trim());
            if w.is_empty() || py.is_empty() {
                continue;
            }
            let Some(code) = shuangpin::encode_pinyin(Scheme::Xiaohe, py) else {
                bad.push(format!("{w}\t{py}\t编码失败"));
                continue;
            };
            total += 1;
            if !lookup(&code, usize::MAX).iter().any(|x| &**x == w) {
                bad.push(format!("{w}\t{py}\tcode={code}"));
            }
        }
        println!("dict.txt {} 条，查不到 {} 条", total, bad.len());
        for b in bad.iter().take(40) {
            println!("  {b}");
        }
    }

    #[test]
    fn lookup_prefix_hits() {
        let _g = scheme_guard();
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
        let _g = scheme_guard();
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
        let _g = scheme_guard();
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
        let _g = scheme_guard();
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
        let _g = scheme_guard();
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
        let _g = scheme_guard();
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
        let _g = scheme_guard();
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
        let _g = scheme_guard();
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
        let _g = scheme_guard();
        set_scheme(Scheme::Xiaohe);
        // `de` 的候选是「的/得/地…」这一批字本身，第一个仍是「的」，
        // 但那是因为静态词频，不是因为一简字（`de` 没有一简字）。
        assert!(crate::brief::char_for("de").is_none());
        let r = lookup("de", 5);
        assert_eq!(r.first().map(|s| &**s), Some("的"));
    }

    #[test]
    fn retroflex_lookup() {
        let _g = scheme_guard();
        set_scheme(Scheme::Xiaohe);
        // 小鹤：shi = ui、zhong = vs
        assert!(lookup("ui", 5).iter().any(|w| &**w == "是"));
        assert!(lookup("vs", 5).iter().any(|w| &**w == "中"));
    }

    #[test]
    fn ms_scheme_lookup() {
        let _g = scheme_guard();
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

    /// 辅助码必须**按官方码**命中：用户是拿小鹤反查工具对出来的码来敲的。
    ///
    /// 带 = 双拼 dd + 形码 fj（官方全码 ddfj，拆「带上 / 巾」）。这一条同时看住了
    /// 两件事：table 里存的是官方码、筛选路径按首字算。曾经这两件事都不成立——
    /// 推算码给的是 sj，用户敲 fj 直接筛空，看起来就是「辅助码没生效」。
    #[test]
    fn xiaohe_assist_hits_official_code() {
        let _g = scheme_guard();
        set_scheme(Scheme::Xiaohe);

        let first_key = lookup_filtered("dd", "f", AssistMode::Xiaohe, 45);
        assert!(
            first_key.iter().any(|w| &**w == "带"),
            "带 应该命中首形 f，实际 {first_key:?}"
        );

        let full = lookup_filtered("dd", "fj", AssistMode::Xiaohe, 45);
        assert_eq!(
            full.first().map(|w| w.as_ref()),
            Some("带"),
            "dd + fj 第一位必须是 带，实际 {full:?}"
        );
    }

    /// 自然码辅助码：带 → 部首 巾 → j
    #[test]
    fn ziranma_assist_hits_radical() {
        let _g = scheme_guard();
        set_scheme(Scheme::Xiaohe);
        let r = lookup_filtered("dd", "j", AssistMode::Ziranma, 45);
        assert!(r.iter().any(|w| &**w == "带"), "自然码 dd + j 应该出 带，实际 {r:?}");
    }

    /// 码表本身不许有垃圾：每位都是 a-z、长度 1~2（自然码 1 位、小鹤 2 位），
    /// 多音字多位用 `|` 分隔，`-` 表示这个方案查不到。
    ///
    /// 写错的码不会报错，只会让筛选**静默失效**——用户敲对了也一片空白，
    /// 所以只能在这一层拦住。小鹤列现在来自官方接口（`tools/xhup_cache.txt`），
    /// 生成脚本改坏 / 缓存被手改，都会在这里掉下来。
    #[test]
    fn assist_table_format_is_sane() {
        let ok = |code: &str| {
            code == "-"
                || code
                    .split('|')
                    .all(|one| !one.is_empty() && one.len() <= 2 && one.chars().all(|c| c.is_ascii_lowercase()))
        };
        let bad: Vec<&str> = assist::RAW
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
            .filter(|l| {
                let cols: Vec<&str> = l.split('\t').collect();
                cols.len() < 3 || cols[1..].iter().any(|code| !ok(code))
            })
            .collect();
        assert!(bad.is_empty(), "辅助码表格式异常：{bad:?}");
    }

    /// 「不是简码，就该单字优先」这条不变量：`collect` 出来的码长必须**非递减**。
    ///
    /// 一简 / 二简 / 自造词 / 置顶是**显式**插队，都发生在 collect 之后，
    /// 所以不在这条断言里——除此之外任何长码词跑到短码字前面都是 bug。
    #[test]
    fn collect_orders_short_codes_first() {
        let _g = scheme_guard();
        for scheme in [Scheme::Xiaohe, Scheme::Ms] {
            set_scheme(scheme);
            for code in ["iu", "de", "dd", "ee", "oo", "he", "zuci", "fwhe"] {
                let (words, lens) = collect(false, code, "", AssistMode::Off, 45);
                assert!(
                    lens.windows(2).all(|w| w[0] <= w[1]),
                    "{scheme:?} 的 {code} 码长顺序乱了：{:?}",
                    words.iter().zip(lens.iter()).collect::<Vec<_>>()
                );
            }
        }
    }


    /// 辅助码筛选**不能**被 `limit` 提前截断。
    ///
    /// 之前 `collect` 先用 `top_by_rank` 取前 `limit` 条再过滤：`ji` 词频最高的
    /// 是「机」，辅助码 `wj` 命中不了它，于是 `limit = 1` 时筛选结果永远为空。
    /// 3 键直出（`jiw`）恰好是拿 `limit = 1` 判断命中的，表现为「勾了 3 码出单字，
    /// 敲 `jiw` 却没反应」；用户按反查工具敲 `jiwj` 也一个字都出不来。
    #[test]
    fn assist_filter_is_not_truncated_by_limit() {
        let _g = scheme_guard();
        assert_eq!(
            crate::assist::code_of('玑', AssistMode::Xiaohe),
            Some("wj"),
            "辅助码表里 玑 必须是官方码 wj，否则整条链都对不上"
        );
        for limit in [1usize, 2, 5, usize::MAX] {
            let hit = lookup_filtered("ji", "wj", AssistMode::Xiaohe, limit);
            assert!(
                hit.iter().any(|w| &**w == "玑"),
                "limit={limit} 时 ji+wj 应该筛出玑，实际 {hit:?}"
            );
        }
        // 只看第一项的那条路（3 键直出的命中判定）也必须看得到东西
        let one = lookup_filtered("ji", "w", AssistMode::Xiaohe, 1);
        assert!(
            one.first().is_some_and(|w| w.chars().count() == 1),
            "ji+w 的第一项必须是单字，实际 {one:?}"
        );
    }

    #[test]
    fn assist_filter_narrows() {
        let _g = scheme_guard();
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
        // 小鹤音形：的→白(b) + 勺(u) = bu，用 bu 精确筛只剩「的」系
        let xh = lookup_filtered("de", "bu", AssistMode::Xiaohe, 5000);
        assert!(xh.iter().any(|w| &**w == "的"), "expected 的 in xiaohe {xh:?}");
        assert!(xh.len() < all.len());
    }

    #[test]
    fn pinyin_lookup_works() {
        let _g = scheme_guard();
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
        let p = repo_src().join("user_dict.txt");
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
        let _g = scheme_guard();
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
        let _g = scheme_guard();
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
        let _g = scheme_guard();
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

    /// 完整辅助码：形码打满时，首字形码**完全相等**的单字提到最前。
    ///
    /// 用户报的现场：3 键直出开着，`jm` + `vo`（简 的官方形码）出来的第一候选
    /// 是「简单」——筛选只按前缀、排序只看词频，精确指定的单字沉在后面。
    /// 用真 assist 表（静态数据，和 `split_trailing_assist_hits_real_table` 同理）。
    #[test]
    fn full_assist_promotes_exact_shape_chars() {
        use crate::assist::AssistMode;

        // 简 的小鹤形码是 vo（assist.txt 官方码），单 的不是
        assert_eq!(crate::assist::code_of('简', AssistMode::Xiaohe), Some("vo"));
        assert_ne!(crate::assist::code_of('单', AssistMode::Xiaohe), Some("vo"));

        let mut out = vec![
            Cow::Borrowed("简单"),
            Cow::Borrowed("简"),
            Cow::Borrowed("单"),
        ];
        promote_exact_shape(&mut out, "vo", AssistMode::Xiaohe, true);
        assert_eq!(
            out.iter().map(|w| w.as_ref()).collect::<Vec<_>>(),
            vec!["简", "简单", "单"],
            "形码打满：精确单字置顶，其余相对顺序不变"
        );

        // 形码没打满 = 前缀筛选语义，不动排序
        let mut out = vec![Cow::Borrowed("简单"), Cow::Borrowed("简")];
        promote_exact_shape(&mut out, "v", AssistMode::Xiaohe, true);
        assert_eq!(out[0], "简单");

        // 多字词的首字即使精确命中也不提（用户打满形码要的是字不是词）
        let mut out = vec![Cow::Borrowed("简报"), Cow::Borrowed("件")];
        promote_exact_shape(&mut out, "vo", AssistMode::Xiaohe, true);
        assert_eq!(out.iter().any(|w| w.as_ref() == "简"), false);

        // 自然码 1 键打满同理：部首码相等的单字置顶（的→白→b）
        let mut out = vec![Cow::Borrowed("地方"), Cow::Borrowed("的")];
        promote_exact_shape(&mut out, "b", AssistMode::Ziranma, true);
        assert_eq!(out[0], "的");

        // 但**两个音节**的查询是「筛词」：自然码只有 1 键，敲第一个字母就算打满，
        // 不设闸的话 `nihc` + `r` 会把单字「你」顶到「你好」前面。
        let mut out = vec![Cow::Borrowed("你好"), Cow::Borrowed("你们"), Cow::Borrowed("你")];
        promote_exact_shape(&mut out, "b", AssistMode::Ziranma, false);
        assert_eq!(out[0], "你好", "多音节查询不该把首字提到词前面");
    }

    /// 模糊音合并时同一个词可能从两条支路都冒出来（`南` 的码是 `nj`，打 `lj` 时
    /// 它既可能在原样结果里、也可能从模糊变体里出现）。去重保留**先出现的**那一份
    /// ——原样支路排在前面，所以精确命中不会被模糊那份顶掉；三个平行数组必须同步删。
    #[test]
    fn dedupe_keeps_first_and_keeps_arrays_in_step() {
        let mut out = vec![
            Cow::Borrowed("南"),
            Cow::Borrowed("兰"),
            Cow::Borrowed("南"),
            Cow::Borrowed("难"),
        ];
        let mut lens = vec![2u8, 2, 4, 2];
        let mut tiers = vec![0u8, 0, 1, 1];
        dedupe_keep_first(&mut out, &mut lens, &mut tiers);
        assert_eq!(
            out.iter().map(|w| w.as_ref()).collect::<Vec<_>>(),
            vec!["南", "兰", "难"]
        );
        assert_eq!(lens, vec![2, 2, 2], "码长必须跟着一起删");
        assert_eq!(tiers, vec![0, 0, 1], "层级必须跟着一起删");
    }

    // ── 输入方式：按键串 → 码（Android 的 26 键靠这一条活着） ──────────
    //
    // 这一组**不查词库**：探针是假的。理由和 `fancha` 那批一样——一旦把
    // `lookup` 的结果写进断言，用户导入个人词库的那天它就红了，而红的位置
    // 指向的会是「输入方式判断错了」，完全误导。

    /// 全拼模式：按键就是全拼，整串编成码。
    #[test]
    fn pinyin_mode_encodes_the_whole_string() {
        let r = resolve_code_with("nihao", true, Scheme::Xiaohe, |_| false).unwrap();
        assert_eq!(r.code, "nihc");
        assert!(r.from_pinyin);

        // 还没打完（`nih`）/ 拼了不存在的音节（`zzz`）→ 没有码
        assert!(resolve_code_with("nih", true, Scheme::Xiaohe, |_| false).is_none());
        assert!(resolve_code_with("zzz", true, Scheme::Xiaohe, |_| false).is_none());
    }

    /// 全拼模式**一次都不问探针**。问它就意味着「又去按双拼解释了」，而用户选全拼
    /// 正是为了「`xian` 出先、不出西安」。
    #[test]
    fn pinyin_mode_never_probes_the_dict() {
        let called = std::cell::Cell::new(false);
        let _ = resolve_code_with("xian", true, Scheme::Xiaohe, |_| {
            called.set(true);
            true
        });
        assert!(!called.get(), "全拼模式不该去问「这个双拼码有没有候选」");
    }

    /// 双拼模式：原样查得到就当码，一个字都不转。
    ///
    /// `xian` 在双拼用户手里是「西 + 安」（西安），在全拼用户手里是「先」——同一个
    /// 串两种意思，**这就是为什么必须有模式开关，而不是自动猜**。
    #[test]
    fn shuangpin_mode_takes_the_code_as_is() {
        let r = resolve_code_with("xian", false, Scheme::Xiaohe, |c| c == "xian").unwrap();
        assert_eq!(r.code, "xian");
        assert!(!r.from_pinyin, "原样当码时不该标成「按全拼解释」");
    }

    /// 双拼模式的兜底：码查不到才按全拼解释。
    ///
    /// 手机上第一次装完默认就是双拼，用户什么设置都没动、直接打 `nihao`——没有这条
    /// 兜底，屏幕会一片空白，然后他会认为「输入法坏了」。
    #[test]
    fn shuangpin_mode_falls_back_to_pinyin() {
        let r = resolve_code_with("nihao", false, Scheme::Xiaohe, |_| false).unwrap();
        assert_eq!(r.code, "nihc");
        assert!(r.from_pinyin);
    }

    /// 空串：不问探针、不给码。
    #[test]
    fn empty_input_asks_nothing() {
        assert!(resolve_code_with("", false, Scheme::Xiaohe, |_| panic!("空串不该查词库")).is_none());
        assert!(resolve_code_with("", true, Scheme::Xiaohe, |_| panic!("空串不该查词库")).is_none());
    }

    /// 编码必须跟当前方案走。**写死 `Xiaohe` 是这里最容易犯的错**：切到微软双拼后
    /// 每个音节的韵母位置都会错，而屏幕上完全看不出来，只表现为「候选莫名其妙不对」。
    #[test]
    fn resolution_follows_the_current_scheme() {
        let _g = scheme_guard();
        for s in [Scheme::Xiaohe, Scheme::Ms] {
            set_scheme(s);
            let expect = shuangpin::encode_phrase(s, "shuang").expect("编码失败");
            let (code, _) = lookup_input("shuang", true, 3);
            assert_eq!(code, expect, "{s:?} 下全拼模式没按当前方案编码");
        }
        set_scheme(Scheme::Xiaohe);
    }

    /// `lookup_input` 在没码时返回空，**不是**返回一个空串码——壳靠「码是不是空」
    /// 决定状态条上要不要显示箭头。
    #[test]
    fn lookup_input_gives_no_code_for_unencodable_input() {
        let _g = scheme_guard();
        set_scheme(Scheme::Xiaohe);
        assert_eq!(lookup_input("", false, 9), (String::new(), Vec::new()));
        assert_eq!(lookup_input("zzz", true, 9), (String::new(), Vec::new()));
    }

    // ── 长串自动分词（手机上第一次有「整句」候选） ──────────────────
    //
    // 假表：`vh`=张、`sj`=三、`li`=李，`vhsj`=张三。所有断言都不碰真词库。

    fn fake_has(c: &str) -> bool {
        matches!(c, "vh" | "sj" | "li" | "vhsj" | "vhsjli")
    }

    fn fake_first(c: &str) -> Option<String> {
        Some(
            match c {
                "vh" => "张",
                "sj" => "三",
                "li" => "李",
                "vhsj" => "张三",
                _ => return None,
            }
            .to_string(),
        )
    }

    /// 该切成 `vhsj|li` 而不是 `vh|sj|li`：能切完的前提下取最长段。
    #[test]
    fn compose_sentence_prefers_the_longest_segments() {
        let s = compose_sentence_with("vhsjli", fake_has, fake_first);
        assert_eq!(s.as_deref(), Some("张三李"));
    }

    /// 缺字就整句放弃。**不许**出现「张三」+ 段码这种混搭上屏。
    #[test]
    fn compose_sentence_gives_up_when_a_segment_has_no_word() {
        // `sj` 在 `has` 里，但 `first` 给不出词（模拟「存在性探测和取第一名不一致」，
        // 或者词在这条路径上查不到）——这时宁可什么都不给。
        let first = |c: &str| if c == "sj" { None } else { fake_first(c) };
        assert_eq!(compose_sentence_with("vhsjli", fake_has, first), None);
    }

    /// 一个音节谈不上分段，硬拼只会把单字当「整句」塞进候选条。
    #[test]
    fn compose_sentence_needs_two_syllables() {
        assert_eq!(compose_sentence_with("vh", fake_has, fake_first), None);
        assert_eq!(compose_sentence_with("vhs", fake_has, fake_first), None, "奇数长度");
    }

    /// 返回的整句**全部是汉字**（这条不假设具体词，但能挡住「混进段码」的回归）。
    #[test]
    fn real_sentence_contains_no_ascii() {
        let _g = scheme_guard();
        set_scheme(Scheme::Xiaohe);
        // 词库里有没有这一串不确定，所以只对这个条件下该成立的结论做断言
        if let Some(s) = compose_sentence("vhsjli") {
            assert!(
                s.chars().all(|c| !c.is_ascii()),
                "整句里混进了编码：{s:?}"
            );
            assert!(s.chars().count() >= 2, "至少两段，结果不该是单字：{s:?}");
        }
    }
}
