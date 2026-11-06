//! 从搜狗拼音导入用户词库（`tools/import_sogou.py` 的运行期版本）。
//!
//! 原来这是开发者在自己机器上跑一次脚本、把结果写进 `src/user_dict.txt` 再重新
//! 编译 DLL。用户换不了自己的词库，等于「从搜狗迁移」这条路只有作者能走。现在
//! 同样的解析逻辑搬进 DLL，设置窗里点一下就把搜狗的打字历史并进个人词库。
//!
//! ## 数据源
//!
//! ```text
//! <搜狗安装目录>/用户数据/SogouPY.users/<id>/sgim_gd_usrbg.bin   打字历史
//! <搜狗安装目录>/用户数据/SogouPY.users/<id>/sgim_gd_usr.bin     用户词库
//! <搜狗安装目录>/Scd/*.scel                                      细胞词库
//! <用户数据目录>/SogouPY/users/<id>/UserDictionary.txt            手动导出
//! ```
//!
//! ## 注音
//!
//! 搜狗的 bin/scel **只有词、没有拼音**，得自己注。策略分两级：
//!
//! 1. 先查编译进 DLL 的基础词库（`dictbin::get()`）——那里是「词 → 全拼」，
//!    多音字天然正确（`重庆` 就该是 `chong qing`）；
//! 2. 查不到才逐字注音，单字取基础词库里的**第一条**读音。基础词库是按词频排的，
//!    同一个字的多条读音里第一条就是最常用的那个。
//!
//! 逐字注音会错（`银行` 可能注成 `yin xing`），但它只作用于基础词库没有的词，
//! 而且错了也只是那个词排得靠后、或者敲错码——不会污染别的词。
//!
//! ## 为什么不用 Python 脚本那套「校验和」逻辑的分歧版本
//!
//! `sgim_gd_usrbg.bin` 每条记录末尾有个 `字节长 + 6` 的字段，可以当校验和用，
//! 能把字节错位产生的假词全部滤掉；`sgim_gd_usr.bin` 没有这个字段，只能宽松
//! 匹配、靠「必须能注音」兜底。两者的解析函数因此分开，`bin_strict` / `bin_loose`。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::shuangpin::{self, Scheme};

/// 收录词的最大字数。6 是脚本里的默认值：再长基本是整句，进词库只会污染候选。
pub const MAX_LEN: usize = 6;

const CJK_MIN: u32 = 0x4E00;
const CJK_MAX: u32 = 0x9FFF;

// ─────────────────────────────────────────────────────────────
// 结果
// ─────────────────────────────────────────────────────────────

/// 一次导入的统计。只留**设置窗要说给人听**的那几个数；更细的（扫描了多少条、
/// 多少走基础词库、多少逐字注音）写进 `feihe.log`，那是排查用的。
pub struct Imported {
    /// 成功解析的文件数
    pub files: usize,
    /// 去重后能注音的词条数
    pub encoded: usize,
    /// 真正新并进个人词库的条数（已经在词库里的不算）
    pub added: usize,
    /// 注不出音、被丢掉的条数
    pub skipped: usize,
}

// ─────────────────────────────────────────────────────────────
// 纯解析（不碰文件，方便单测）
// ─────────────────────────────────────────────────────────────

fn is_all_cjk(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| (CJK_MIN..=CJK_MAX).contains(&(c as u32)))
}

fn count_chars(s: &str) -> usize {
    s.chars().count()
}

fn u16_at(data: &[u8], p: usize) -> Option<u16> {
    let b = data.get(p..p + 2)?;
    Some(u16::from_le_bytes([b[0], b[1]]))
}

fn utf16_at(data: &[u8], p: usize, bytes: usize) -> Option<String> {
    let slice = data.get(p..p + bytes)?;
    let mut units = Vec::with_capacity(bytes / 2);
    for c in slice.chunks_exact(2) {
        units.push(u16::from_le_bytes([c[0], c[1]]));
    }
    String::from_utf16(&units).ok()
}

/// `sgim_gd_usrbg.bin`：每条记录 `[u16 字节长][UTF-16LE 文本][u16 字节长+6]`。
///
/// 第三个字段是校验和，用它把字节错位产生的假词滤掉——这是唯一能可靠判定的
/// 地方，否则按 2 字节步长扫整个文件会捞出一大堆「看着像汉字」的垃圾。
fn extract_bin_strict(data: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let n = data.len();
    let mut p = 0;
    while p + 6 <= n {
        if let Some(blen) = u16_at(data, p) {
            let blen = blen as usize;
            if blen >= 4 && blen <= 32 && blen % 2 == 0 && p + 2 + blen + 2 <= n {
                let end = p + 2 + blen;
                if u16_at(data, end) == Some(blen as u16 + 6) {
                    if let Some(text) = utf16_at(data, p + 2, blen) {
                        if is_all_cjk(&text) {
                            out.push(text);
                        }
                    }
                }
            }
        }
        p += 2;
    }
    out
}

/// `sgim_gd_usr.bin`：没有校验和，只能「[u16 偶数长度][全 CJK 文本]」宽松匹配。
///
/// 噪声靠后面的「必须能注音」过滤——乱码基本注不出音，注得出来的假词也就混进去
/// 几个，比丢掉整个文件划算。
fn extract_bin_loose(data: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let n = data.len();
    let mut p = 0;
    while p + 4 <= n {
        if let Some(blen) = u16_at(data, p) {
            let blen = blen as usize;
            if blen >= 4 && blen <= 32 && blen % 2 == 0 && p + 2 + blen <= n {
                if let Some(text) = utf16_at(data, p + 2, blen) {
                    if is_all_cjk(&text) {
                        out.push(text);
                    }
                }
            }
        }
        p += 2;
    }
    out
}

/// 搜狗细胞词库 `.scel`。
///
/// ```text
/// "SougouPY" 0x0A [4 个以 0x0A00 结尾的 UTF-16LE 头字段] 词条区
/// 词条： [u16 拼音字符数][拼音 UTF-16LE][u16 词字符数][词 UTF-16LE]
/// ```
///
/// 各版本头字段数略有差异，所以从几个候选起点里挑「解析出词最多」的那个——
/// 这也是脚本当初的做法，实测比自己猜版本号可靠。
fn extract_scel(data: &[u8]) -> Vec<String> {
    if !data.starts_with(b"SougouPY") {
        return Vec::new();
    }
    let mut pos = 9usize;
    for _ in 0..4 {
        while pos + 1 < data.len() && data[pos..pos + 2] != [0x0a, 0x00] {
            pos += 2;
        }
        pos += 2;
    }
    if pos >= data.len() {
        return Vec::new();
    }

    let parse = |start: usize| -> Vec<String> {
        let mut out = Vec::new();
        let mut p = start;
        while p + 4 <= data.len() {
            let Some(plen) = u16_at(data, p).map(|v| v as usize) else { break };
            if plen == 0 || plen > 200 {
                break;
            }
            p += 2;
            if p + plen * 2 + 2 > data.len() {
                break;
            }
            p += plen * 2;
            let Some(wlen) = u16_at(data, p).map(|v| v as usize) else { break };
            if wlen == 0 || wlen > 64 {
                break;
            }
            p += 2;
            if p + wlen * 2 > data.len() {
                break;
            }
            match utf16_at(data, p, wlen * 2) {
                Some(w) if is_all_cjk(&w) => out.push(w),
                Some(_) => {}
                None => break,
            }
            p += wlen * 2;
        }
        out
    };

    let mut best = Vec::new();
    for start in [pos, pos + 4, pos + 2, pos + 8] {
        let got = parse(start);
        if got.len() > best.len() {
            best = got;
        }
    }
    best
}

/// 文本词库：每行 `词` / `词<TAB>拼音` / `词 拼音`，`#` 开头是注释。
fn extract_txt(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split(['\t', ' ']).next())
        .map(str::trim)
        .filter(|w| is_all_cjk(w))
        .map(str::to_string)
        .collect()
}

/// 文本文件解码：搜狗导出的 txt 有 UTF-16LE（带 BOM）也有 UTF-8，两种都认。
///
/// 只按 UTF-8 硬读会把 UTF-16 的「词库」读成一堆 `\0`，然后静默导入 0 条——
/// 用户看到的是「导入了，但什么都没变」，最难查的一种。
fn decode_text(data: &[u8]) -> String {
    if data.starts_with(&[0xFF, 0xFE]) {
        let mut units = Vec::with_capacity(data.len() / 2);
        for c in data[2..].chunks_exact(2) {
            units.push(u16::from_le_bytes([c[0], c[1]]));
        }
        return String::from_utf16_lossy(&units);
    }
    if data.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(&data[3..]).into_owned();
    }
    String::from_utf8_lossy(data).into_owned()
}

/// 把一批词计进汇总表：`词 → (优先级, 出现次数)`。
///
/// 优先级**大的赢**（打字历史 3 > 用户词库 2 > 细胞词库 1），同优先级按出现次数排。
///
/// 注意方向：数字越大越优先，和「先写 1 表示第一」的直觉相反。当初脚本里写的是
/// 「1 = usrbg」，然后按 `-prio` 升序排——结果细胞词库排在打字历史前面，是个
/// 一直没人发现的 bug（词干都在，只是顺序拧了）。这里直接让数字和优先级同向，
/// `tally` 里 `max`、排序里 `Reverse` 都不用人再想一遍。
/// 只收 2..=max_len 个字的词——单字词在搜狗里边是噪声（用户没主动造过单字），
/// 太长的基本是整句。
fn tally_words(
    tally: &mut HashMap<String, (u8, u32)>,
    words: &[String],
    prio: u8,
    max_len: usize,
) {
    for w in words {
        let n = count_chars(w);
        if n < 2 || n > max_len {
            continue;
        }
        match tally.get_mut(w) {
            Some(e) => {
                e.0 = e.0.max(prio);
                e.1 += 1;
            }
            None => {
                tally.insert(w.clone(), (prio, 1));
            }
        }
    }
}

/// 排序：优先级高的、出现次数多的、词短的在前。
///
/// 顺序最终会变成 `user_dict.txt` 的行号，而行号在 `dict::top_by_rank` 里就是
/// 同码长候选的次要排序键——所以「常用词排在文件前面」这件事是有实际效果的。
fn order_rows(rows: &mut [(String, String, u8, u32)]) {
    rows.sort_by_key(|(w, _, prio, cnt)| {
        (std::cmp::Reverse(*prio), std::cmp::Reverse(*cnt), count_chars(w))
    });
}

// ─────────────────────────────────────────────────────────────
// 注音
// ─────────────────────────────────────────────────────────────

struct Encoder {
    /// 词 → 全拼（基础词库里的原文，多音字正确）
    word_py: HashMap<String, String>,
    /// 单字 → 最常用读音
    char_py: HashMap<char, String>,
    from_base: usize,
    from_char: usize,
    skipped: usize,
}

impl Encoder {
    /// 从**编译进 DLL 的基础词库**建表（不是 `src/dict.txt`：运行期没有那个文件）。
    ///
    /// 基础词库里同一个字可能有多条（`长 chang` / `长 zhang`），按词频排在前面，
    /// 所以 `or_insert` 取第一条就是「最常用读音」。
    fn new() -> Self {
        let bin = crate::dictbin::get();
        let mut word_py = HashMap::with_capacity(bin.count());
        let mut char_py = HashMap::new();
        for i in 0..bin.count() {
            let w = bin.word(i);
            let py = bin.pinyin(i);
            if w.is_empty() || py.is_empty() {
                continue;
            }
            word_py.entry(w.to_string()).or_insert_with(|| py.to_string());
            if count_chars(w) == 1 {
                if let Some(c) = w.chars().next() {
                    char_py.entry(c).or_insert_with(|| py.to_string());
                }
            }
        }
        Self {
            word_py,
            char_py,
            from_base: 0,
            from_char: 0,
            skipped: 0,
        }
    }

    fn encode(&mut self, word: &str) -> Option<String> {
        if let Some(py) = self.word_py.get(word).cloned() {
            self.from_base += 1;
            return Some(py);
        }
        let mut syls = Vec::with_capacity(count_chars(word));
        for ch in word.chars() {
            match self.char_py.get(&ch) {
                Some(p) => syls.push(p.as_str()),
                None => {
                    self.skipped += 1;
                    return None;
                }
            }
        }
        let py = syls.join(" ");
        // 校验：这个拼音得在小鹤下编得出完整码，否则这词进来也查不到
        match shuangpin::encode_pinyin(Scheme::Xiaohe, &py) {
            Some(code) if code.len() == 2 * count_chars(word) => {
                self.from_char += 1;
                Some(py)
            }
            _ => {
                self.skipped += 1;
                None
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────
// 发现搜狗数据
// ─────────────────────────────────────────────────────────────

/// 每种文件怎么解析、优先级多少
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    BinStrict,
    BinLoose,
    Scel,
    Text,
}

struct Job {
    label: String,
    path: PathBuf,
    kind: Kind,
    prio: u8,
}

/// 读注册表默认值（搜狗把自己的安装目录存在 `SogouInput` 键的默认值里）。
///
/// 三个位置都试：64 位视图看不到 32 位安装的键，反过来也一样，而搜狗是 32 位
/// 程序、绝大多数人装在 WOW6432Node 下。
fn reg_install_dir(hive: isize, subkey: &str) -> Option<PathBuf> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegGetValueW, HKEY, RRF_RT_REG_SZ,
    };

    let key: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * std::mem::size_of::<u16>()) as u32;
    let rc = unsafe {
        RegGetValueW(
            HKEY(hive as *mut core::ffi::c_void),
            PCWSTR(key.as_ptr()),
            PCWSTR::null(),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut core::ffi::c_void),
            Some(&mut size),
        )
    };
    if rc != ERROR_SUCCESS {
        return None;
    }
    let n = ((size as usize) / 2).min(buf.len());
    let s = String::from_utf16_lossy(&buf[..n]);
    let s = s.trim_end_matches('\u{0}');
    let p = PathBuf::from(s);
    p.is_dir().then_some(p)
}

/// 可能存放搜狗数据的根目录（去重、只留真实存在的）。
fn roots() -> Vec<(String, PathBuf)> {
    // HKEY_LOCAL_MACHINE = 0x80000002，HKEY_CURRENT_USER = 0x80000001
    const HKLM: isize = 0x8000_0002u32 as i32 as isize;
    const HKCU: isize = 0x8000_0001u32 as i32 as isize;

    let mut cands: Vec<(String, PathBuf)> = Vec::new();
    for (hive, tag, sub) in [
        (HKLM, "HKLM", r"SOFTWARE\WOW6432Node\SogouInput"),
        (HKLM, "HKLM", r"SOFTWARE\SogouInput"),
        (HKCU, "HKCU", r"SOFTWARE\SogouInput"),
    ] {
        if let Some(p) = reg_install_dir(hive, sub) {
            cands.push((tag.to_string(), p));
        }
    }
    for env in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Ok(base) = std::env::var(env) {
            cands.push((env.to_string(), Path::new(&base).join("SogouInput")));
        }
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        cands.push(("APPDATA".into(), Path::new(&appdata).join("SogouPY")));
        cands.push(("APPDATA".into(), Path::new(&appdata).join("SogouPY.users")));
        cands.push(("APPDATA".into(), Path::new(&appdata).join("SogouInput")));
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        cands.push(("LOCALAPPDATA".into(), Path::new(&local).join("SogouPY")));
        cands.push(("LOCALAPPDATA".into(), Path::new(&local).join("SogouPY.users")));
    }

    let mut seen = std::collections::HashSet::new();
    cands
        .into_iter()
        .filter(|(_, p)| p.is_dir())
        .filter(|(_, p)| seen.insert(p.clone()))
        .collect()
}

fn dir_has_sgim(dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else { return false };
    rd.flatten()
        .any(|e| e.file_name().to_string_lossy().starts_with("sgim_"))
}

/// 一个「用户数据目录」里的文件（`sgim_gd_usrbg.bin` / `sgim_gd_usr.bin` / 导出的 txt）
fn jobs_in_user_dir(dir: &Path, label: &str, out: &mut Vec<Job>) {
    let bg = dir.join("sgim_gd_usrbg.bin");
    if bg.is_file() {
        out.push(Job {
            label: format!("{label}/usrbg"),
            path: bg,
            kind: Kind::BinStrict,
            prio: 3,
        });
    }
    let usr = dir.join("sgim_gd_usr.bin");
    if usr.is_file() {
        out.push(Job {
            label: label.to_string(),
            path: usr,
            kind: Kind::BinLoose,
            prio: 2,
        });
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let low = name.to_lowercase();
        if low.ends_with(".txt") && (low.contains("dict") || name.contains("词库")) {
            out.push(Job {
                label: format!("{label}/{name}"),
                path: e.path(),
                kind: Kind::Text,
                prio: 1,
            });
        }
    }
}

fn jobs_in_scd_dir(dir: &Path, label: &str, out: &mut Vec<Job>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.to_lowercase().ends_with(".scel") {
            out.push(Job {
                label: format!("{label}/{name}"),
                path: e.path(),
                kind: Kind::Scel,
                prio: 1,
            });
        }
    }
}

/// 自动发现：从注册表 / 环境变量推搜狗的安装目录和用户数据目录。
fn discover() -> Result<(Vec<(String, PathBuf)>, Vec<Job>), String> {
    let roots = roots();
    if roots.is_empty() {
        return Err(format!(
            "没找到搜狗输入法。\n\n如果它装在别的盘，可以用「选择文件夹…」手动指定，\
             一般是：\n<搜狗安装目录>\\用户数据\\SogouPY.users\\<一串数字>"
        ));
    }
    let mut jobs = Vec::new();
    for (label, root) in &roots {
        // 版本之间层级不一样：有的直接是用户目录，有的下面还套 SogouPY.users/<id>。
        // 先试「root 自己就是」，再试三层相对路径——猜错层级只是多几次 stat。
        if dir_has_sgim(root) {
            jobs_in_user_dir(root, label, &mut jobs);
        }
        // <root>/用户数据/SogouPY.users/<id>/
        for rel in ["用户数据\\SogouPY.users", "users", "SogouPY.users"] {
            let base = root.join(rel);
            let Ok(rd) = std::fs::read_dir(&base) else { continue };
            for e in rd.flatten() {
                let sub = e.path();
                if sub.is_dir() && dir_has_sgim(&sub) {
                    let tag = format!("{label}:{rel}/{}", e.file_name().to_string_lossy());
                    jobs_in_user_dir(&sub, &tag, &mut jobs);
                }
            }
        }
        let scd = root.join("Scd");
        if scd.is_dir() {
            jobs_in_scd_dir(&scd, &format!("{label}:Scd"), &mut jobs);
        }
    }
    Ok((roots, jobs))
}

/// 手动指定的目录：既可能是「用户数据目录」本身，也可能是它的上一层。
///
/// 不假设层级——把目录自己、它的子目录、以及 `Scd` 都扫一遍，用户选错一层也
/// 能导入，比弹一个「这里没有搜狗数据」有用得多。
fn jobs_in_dir(dir: &Path) -> Vec<Job> {
    let mut jobs = Vec::new();
    if dir_has_sgim(dir) {
        jobs_in_user_dir(dir, &dir.display().to_string(), &mut jobs);
    }
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let sub = e.path();
            if sub.is_dir() && dir_has_sgim(&sub) {
                jobs_in_user_dir(&sub, &sub.display().to_string(), &mut jobs);
            }
        }
    }
    jobs_in_scd_dir(dir, &dir.display().to_string(), &mut jobs);
    let scd = dir.join("Scd");
    if scd.is_dir() {
        jobs_in_scd_dir(&scd, &format!("{}/Scd", dir.display()), &mut jobs);
    }
    jobs
}

// ─────────────────────────────────────────────────────────────
// 主流程
// ─────────────────────────────────────────────────────────────

fn run(jobs: &[Job], max_len: usize) -> Result<Imported, String> {
    if jobs.is_empty() {
        return Err("没有发现可解析的搜狗词库文件".into());
    }

    let mut tally: HashMap<String, (u8, u32)> = HashMap::new();
    let mut files = 0usize;
    let mut scanned = 0usize;
    for job in jobs {
        let Ok(data) = std::fs::read(&job.path) else { continue };
        let words = match job.kind {
            Kind::BinStrict => extract_bin_strict(&data),
            Kind::BinLoose => extract_bin_loose(&data),
            Kind::Scel => extract_scel(&data),
            Kind::Text => extract_txt(&decode_text(&data)),
        };
        files += 1;
        scanned += words.len();
        // 一个文件一行：用户报「导入后词没变多」时，这份日志能直接看出是
        // 哪个文件没解析出词（搜狗版本一升，bin 的布局就可能变）。
        crate::diag::log(format_args!("sogou: {} → {} 条", job.label, words.len()));
        tally_words(&mut tally, &words, job.prio, max_len);
    }

    let mut enc = Encoder::new();
    let mut rows: Vec<(String, String, u8, u32)> = Vec::with_capacity(tally.len());
    for (word, (prio, cnt)) in tally {
        if let Some(py) = enc.encode(&word) {
            rows.push((word, py, prio, cnt));
        }
    }
    order_rows(&mut rows);

    let mut text = String::with_capacity(rows.len() * 24);
    for (word, py, _, _) in &rows {
        text.push_str(word);
        text.push('\t');
        text.push_str(py);
        text.push('\n');
    }

    let (added, _) = crate::dict::merge_text(&text)?;
    crate::diag::log(format_args!(
        "sogou: 解析 {files} 个文件 / {scanned} 条 → 可注音 {} 条（词库 {}、逐字 {}），\
         跳过 {}，新增 {added}",
        rows.len(),
        enc.from_base,
        enc.from_char,
        enc.skipped
    ));
    Ok(Imported {
        files,
        encoded: rows.len(),
        added,
        skipped: enc.skipped,
    })
}

/// 自动发现并导入。
pub fn import_auto(max_len: usize) -> Result<Imported, String> {
    let (_roots, jobs) = discover()?;
    if jobs.is_empty() {
        return Err(
            "找到了搜狗目录，但里面没有用户词库文件。\n\n可能的原因：一次都没用搜狗打过字，\
             或者它还没把词库落盘。"
                .into(),
        );
    }
    run(&jobs, max_len)
}

/// 用户手动挑的一个词库文件 → 解析任务。扩展名决定解析器。
///
/// 优先级给 1（细胞词库级）：手动挑的文件和自动发现的细胞词库都是「词库作者
/// 给的词」，没有谁更可信；打字历史（3）/用户词库（2）永远压在它们上面。
fn job_for_file(path: &Path) -> Result<Job, String> {
    let label = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let kind = match ext.as_str() {
        "scel" => Kind::Scel,
        // 手动挑的 .bin 没有背景/用户之分（那两个文件名是自动发现认的），
        // 用宽松解析兜底：宁可少收几条，不要因为校验和整包拒绝。
        "bin" => Kind::BinLoose,
        "txt" => Kind::Text,
        other => {
            return Err(format!(
                "不认识的扩展名 .{other}（支持 .scel 细胞词库 / .bin / .txt）"
            ))
        }
    };
    Ok(Job {
        label,
        path: path.to_path_buf(),
        kind,
        prio: 1,
    })
}

/// 导入用户手动挑选的**单个文件**——典型场景：从搜狗词库网站下载了一个
/// 细胞词库 .scel，跟本机装没装搜狗完全无关。
pub fn import_file(path: &Path, max_len: usize) -> Result<Imported, String> {
    let job = job_for_file(path)?;
    run(std::slice::from_ref(&job), max_len)
}

/// 从用户手动指定的目录导入。
pub fn import_dir(dir: &Path, max_len: usize) -> Result<Imported, String> {
    let jobs = jobs_in_dir(dir);
    if jobs.is_empty() {
        return Err(format!(
            "{} 里没有搜狗词库文件（找 sgim_*.bin / *.scel / 导出的 txt）。",
            dir.display()
        ));
    }
    run(&jobs, max_len)
}

// ─────────────────────────────────────────────────────────────
// 测试
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 拼一条 `usrbg` 记录：`[u16 字节长][UTF-16LE][u16 字节长+6]`
    fn usrbg_record(word: &str) -> Vec<u8> {
        let units: Vec<u16> = word.encode_utf16().collect();
        let blen = (units.len() * 2) as u16;
        let mut b = Vec::new();
        b.extend_from_slice(&blen.to_le_bytes());
        for u in &units {
            b.extend_from_slice(&u.to_le_bytes());
        }
        b.extend_from_slice(&(blen + 6).to_le_bytes());
        b
    }

    #[test]
    fn bin_strict_accepts_only_checksummed_records() {
        let mut data = usrbg_record("你好");
        data.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef]); // 中间插一段垃圾
        data.extend_from_slice(&usrbg_record("测试"));

        let got = extract_bin_strict(&data);
        assert!(got.contains(&"你好".to_string()), "{got:?}");
        assert!(got.contains(&"测试".to_string()), "{got:?}");
    }

    /// 校验位不对就必须丢掉——这正是「宽松匹配」会产生假词的地方。
    #[test]
    fn bin_strict_rejects_bad_checksum() {
        let mut record = usrbg_record("你好");
        let n = record.len();
        record[n - 1] = 0xFF;
        assert!(extract_bin_strict(&record).is_empty());
        // 但宽松解析本来就只看长度，照样认（不然 usr.bin 一个词都读不出来）
        assert_eq!(extract_bin_loose(&record), vec!["你好".to_string()]);
    }

    #[test]
    fn job_for_file_maps_by_extension() {
        let j = job_for_file(Path::new("下载/经济学.scel")).unwrap();
        assert!(matches!(j.kind, Kind::Scel));
        assert_eq!(j.prio, 1, "手动挑的文件和细胞词库同级");
        assert!(matches!(job_for_file(Path::new("a/b.txt")).unwrap().kind, Kind::Text));
        assert!(matches!(job_for_file(Path::new("a/b.BIN")).unwrap().kind, Kind::BinLoose), "扩展名大小写不敏感");
        assert!(job_for_file(Path::new("a/b.exe")).is_err(), "不认识的扩展名要拒绝而不是瞎猜");
    }

    #[test]
    fn scel_needs_the_sogou_magic() {
        assert!(extract_scel(b"not a scel file").is_empty());
    }

    #[test]
    fn txt_lines_keep_only_cjk_words() {
        let text = "# 注释\n\n你好\tabc\n世界 拼音\nhello\n单\n";
        assert_eq!(
            extract_txt(text),
            vec!["你好".to_string(), "世界".to_string(), "单".to_string()]
        );
    }

    #[test]
    fn decode_text_handles_both_encodings() {
        let mut utf16 = vec![0xFF, 0xFE];
        for u in "词库".encode_utf16() {
            utf16.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(decode_text(&utf16), "词库");

        let mut utf8 = vec![0xEF, 0xBB, 0xBF];
        utf8.extend_from_slice("词库".as_bytes());
        assert_eq!(decode_text(&utf8), "词库");

        assert_eq!(decode_text("词库".as_bytes()), "词库");
    }

    /// 只收 2..=N 个字的词；同词跨文件合并次数、取最高优先级。
    #[test]
    fn tally_merges_counts_and_priority() {
        let mut t = HashMap::new();
        let a: Vec<String> = ["你好", "单", "一个字太多一个字太多"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        tally_words(&mut t, &a, 2, 6);
        assert_eq!(t.get("你好"), Some(&(2, 1)));
        assert!(t.get("单").is_none(), "单字词不进词库");
        assert_eq!(t.len(), 1, "超长词被丢掉：{t:?}");

        let b: Vec<String> = vec!["你好".to_string()];
        tally_words(&mut t, &b, 1, 6);
        assert_eq!(t.get("你好"), Some(&(2, 2)), "次数累加、优先级取最高");
    }

    #[test]
    fn order_rows_prefers_priority_then_count_then_short() {
        let mut rows = vec![
            ("丙".into(), "bing".into(), 1u8, 1u32), // 细胞词库，最低
            ("甲乙丙".into(), "jia yi bing".into(), 3, 1),
            ("乙丙".into(), "yi bing".into(), 3, 4),
            ("甲乙".into(), "jia yi".into(), 3, 5),
        ];
        order_rows(&mut rows);
        let order: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
        assert_eq!(order, vec!["甲乙", "乙丙", "甲乙丙", "丙"]);
    }
}
