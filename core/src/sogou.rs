//! 搜狗词库的**纯解析与注音**：词库文件 → 一批能注音的 `词<TAB>全拼`。
//!
//! 这个模块不碰文件、不碰注册表、不认识「搜狗装在哪个目录」。它只做两件事：
//!
//! 1. 把一堆字节（`.scel` 细胞词库 / `sgim_gd_usr*.bin` / 导出的 `.txt`）解析成词；
//! 2. 给这些词注上全拼，合成一份 `词<TAB>全拼` 文本。
//!
//! 文件从哪来由壳决定：Windows 壳在 `src/sogou.rs` 里用注册表发现搜狗目录，
//! Android 壳从系统文件选择器拿到一个 Uri。**解析和注音只有这一份**——两边各写
//! 一遍 `extract_scel`，迟早会有一边不认新版本的文件格式。
//!
//! 注音策略分两级（和搬进来之前完全一样）：
//!
//! 1. 先查编译进基础词库的「词 → 全拼」，多音字天然正确（`重庆` 就该是 `chong qing`）；
//! 2. 查不到才逐字注音，单字取基础词库里的**第一条**读音。
//!
//! 逐字注音会错（`银行` 可能注成 `yin xing`），但它只作用于基础词库没有的词，
//! 错了也只是那个词排得靠后，不会污染别的词。

use std::collections::HashMap;

use crate::shuangpin::{self, Scheme};

/// 收录词的最大字数。6 是脚本里的默认值：再长基本是整句，进词库只会污染候选。
pub const MAX_LEN: usize = 6;

const CJK_MIN: u32 = 0x4E00;
const CJK_MAX: u32 = 0x9FFF;

/// 文件格式。由壳根据扩展名（或自动发现时的文件名）判定。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `sgim_gd_usrbg.bin`，带校验和，能严格解析
    BinStrict,
    /// `sgim_gd_usr.bin`（以及手动挑的任意 .bin），无校验和，宽松匹配
    BinLoose,
    /// `*.scel` 细胞词库
    Scel,
    /// 导出的 `.txt`（UTF-8 或 UTF-16LE 都认）
    Text,
}

/// 一份待解析的词库文件。
pub struct Job {
    /// 给日志用的标签（文件名或 `目录/文件名`）
    pub label: String,
    /// 文件字节（壳负责读盘；读不出来时壳应当跳过这个 Job，而不是塞空数据）
    pub data: Vec<u8>,
    pub kind: Kind,
    /// 优先级：打字历史 3 > 用户词库 2 > 细胞词库 1。数字大的赢。
    pub prio: u8,
}

// ─────────────────────────────────────────────────────────────
// 结果
// ─────────────────────────────────────────────────────────────

/// 一次导入的统计。只留**设置窗要说给人听**的那几个数；更细的（扫描了多少条、
/// 多少走基础词库、多少逐字注音）写进 `feihe.log`，那是排查用的。
#[derive(Debug, Clone, Default)]
pub struct Imported {
    /// 成功解析的文件数
    pub files: usize,
    /// 解析出来的原始词条数（去重、注音之前）
    pub scanned: usize,
    /// 去重后能注音的词条数
    pub encoded: usize,
    /// 真正新并进个人词库的条数（已经在词库里的不算）。
    /// `encode_*` 出来时是 0，由调用方 `merge_text` 之后填。
    pub added: usize,
    /// 注不出音、被丢掉的条数
    pub skipped: usize,
    /// 走基础词库整词注音的有多少条
    pub from_base: usize,
    /// 逐字注音的有多少条（会错，但只错自己）
    pub from_char: usize,
}

impl Imported {
    /// 一句能给用户看的中文摘要。Windows 的 MessageBox 和 Android 的 Toast 共用。
    ///
    /// 特意把「解析出多少 / 注得了音多少 / 新增多少」都写出来：只说「导入成功」
    /// 的话，碰上「文件格式变了、一条都没解析出来」用户完全没法判断。
    pub fn summary(&self) -> String {
        if self.encoded == 0 {
            return format!(
                "没能从这个词库文件里解析出词（扫了 {} 条）。\n\n                 可能是词库版本太新、格式对不上，或者文件本身损坏。",
                self.scanned
            );
        }
        let mut s = format!(
            "读入 {} 个文件，解析 {} 条，能注音 {} 条\n新增 {} 条到个人词库",
            self.files, self.scanned, self.encoded, self.added
        );
        if self.skipped > 0 {
            s.push_str(&format!("，{} 条注不出音被跳过", self.skipped));
        }
        if self.added == 0 {
            s.push_str("\n\n（这些词本来就在词库里，所以没有新增）");
        }
        s
    }
}

// ─────────────────────────────────────────────────────────────
// 纯解析（不碰文件，方便单测）
// ─────────────────────────────────────────────────────────────

/// 是不是「纯小写 ASCII 字母」的音节（`ai` / `zhang`）。`.scel` 的音节表就是这个形状。
fn is_ascii_syllable(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase())
}

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
    // 新格式**没有** `SougouPY` magic（2026-11 的搜狗导出直接以 `DCS` 风格头开始），
    // 所以先试新解析器，再退回老格式。以前这里先判 magic，导致真文件在第一步就
    // 返回空——那正是「导入没反应」的直接原因。
    if let Some(w) = parse_scel_modern(data) {
        return w;
    }
    if !data.starts_with(b"SougouPY") {
        return Vec::new();
    }
    parse_scel_legacy(data)
}

/// 现代搜狗 `.scel`（实测样本：`中国风景名胜.scel`，53 KB，2026-11 的导出格式）。
///
/// **实测出来的布局**（不是猜的，按字节对着 `15508`「中国风景名胜」这个词库解出来的）：
///
/// ```text
/// 0      : u32 = 4
/// 4      : u16 len + UTF-16  "DCS\x01\x01..."（风格名）
/// ...    : u16 len + UTF-16  拼音方案名 "11508"（和编号同一段里）
/// ...    : 一大段 0 填充
/// S      : 音节表 —— 连续 [u16 序号][u16 字节长][ascii]，412 条（ai / an / ang … zuo）
/// S+end  : u16=1, [u16 字节长 + 索引...]   ← 一小段「示例索引」，跳过
/// 词条区 : 每条 = [u16 词字节长][词 UTF-16]
///                  [u16=1][u16 拼音字节长][u16 拼音索引...]      ← 拼音 A（高 2 位是标记）
///                  [u16 id][5×u16][u16=1][u16 拼音字节长][索引...] ← 拼音 B / 词频等
/// ```
///
/// 拼音索引的低 14 位是音节表下标 **+1**（0 = 空）。
///
/// 老格式见 `parse_scel_legacy`——它要求文件以 `SougouPY` 开头，而现在的搜狗
/// 客户端不写那个 magic 了，所以老解析器在真文件上一个词都读不出来。
fn parse_scel_modern(data: &[u8]) -> Option<Vec<String>> {
        let mut syl: Vec<String> = Vec::new();
    let syl_start = find_syllable_table(data, 2)?;
    let mut p = syl_start;
    while p + 4 <= data.len() {
        let Some(ln) = u16_at(data, p + 2) else { break };
        let ln = ln as usize;
        if ln == 0 || ln > 16 || p + 4 + ln > data.len() {
            break;
        }
            let Some(s) = utf16_at(data, p + 4, ln) else { break };
        if !is_ascii_syllable(&s) {
            break;
        }
        syl.push(s);
        p += 4 + ln;
    }
    if syl.len() < 100 {
        return None; // 不像音节表，退回老解析器
    }


    // 词条区：逐 2 字节找「像记录」的位置，命中了就按结构前进，没命中就跳 2。
    // 记录之间夹杂的块（示例索引、分段表）都靠这条规则跳过——**绝不整包放弃**。
    let mut words = Vec::new();
    let mut q = p;
    let mut misses = 0usize;
    while q + 6 <= data.len() {
        match parse_scel_record(data, q, syl.len()) {
            Some((w, next)) => {
                words.push((w, q));
                q = next;
                misses = 0;
            }
            None => {
                q += 2;
                misses += 1;
                if words.is_empty() && misses > 20_000 {
                    return None;
                }
            }
        }
    }
    if words.is_empty() {
        return None;
    }
    Some(words.into_iter().map(|(w, _)| w).collect())
}

/// 解析一条现代格式的词条。返回 `(词, 下一条的起点)`。
///
/// 实测（`中国风景名胜.scel`）词条的最小形态是：
///
/// ```text
/// [u16 词字节长][词 UTF-16]         ← 词语本体
/// [u16 id][5×u16][u16=1][u16 字节长][u16 拼音索引...]   ← 尾巴 12+4+N 字节
/// ```
///
/// 我们只取「词」。拼音索引的合法性用来判断「这个位置真的是一条记录」，
/// 否则按 2 字节扫的时候会到处命中假记录。
fn parse_scel_record(data: &[u8], q: usize, syl_count: usize) -> Option<(String, usize)> {
    let wl = u16_at(data, q)? as usize;
    if wl < 4 || wl > 128 || wl % 2 != 0 || q + 2 + wl > data.len() {
        return None;
    }
    let w = utf16_at(data, q + 2, wl)?;
    if !is_all_cjk(&w) {
        return None;
    }
    let body_end = q + 2 + wl;

    let pinyin_ok = |at: usize| -> Option<usize> {
        // 期望 [u16=1][u16 字节长][索引...]
        if u16_at(data, at)? != 1 {
            return None;
        }
        let ln = u16_at(data, at + 2)? as usize;
        if ln < 2 || ln > 64 || ln % 2 != 0 || at + 4 + ln > data.len() {
            return None;
        }
        for k in 0..ln / 2 {
            let v = u16_at(data, at + 4 + k * 2)? & 0x3FFF;
            let i = v as usize;
            if i < 1 || i > syl_count {
                return None;
            }
        }
        Some(at + 4 + ln)
    };

    // 形态 A：紧跟 12 字节尾巴头（id + 5×u16），再是拼音段
    if body_end + 10 <= data.len() {
        if let Some(next) = pinyin_ok(body_end + 12) {
            return Some((w, next));
        }
    }
    // 形态 B：拼音段紧跟在词后面（老式记录，`[词][拼音]`）
    if let Some(next) = pinyin_ok(body_end) {
        return Some((w, next));
    }
    None
}

/// 找音节表起点：`[u16 序号][u16 字节长][小写 ascii]` 连着出现 ≥ `min` 次。
///
/// 从文件头逐 2 字节扫（不能跳，因为头部长度随版本变），找到第一个「能连读很多条」
/// 的位置就认定是音节表。样本停在 412 条。
fn find_syllable_table(data: &[u8], min: usize) -> Option<usize> {
    let mut best: Option<(usize, usize)> = None;
    let mut p = 4usize;
    while p + 4 < data.len() {
        let mut q = p;
        let mut n = 0usize;
        while q + 4 < data.len() && n < 500 {
            let Some(idx) = u16_at(data, q) else { break };
            let Some(ln) = u16_at(data, q + 2) else { break };
            let ln = ln as usize;
            if idx == 0 || ln < 2 || ln > 16 || q + 4 + ln > data.len() {
                break;
            }
            match utf16_at(data, q + 4, ln) {
                Some(s) if is_ascii_syllable(&s) => {}
                _ => break,
            }
            n += 1;
            q += 4 + ln;
        }
        if n >= min && best.map(|(_, b)| n > b).unwrap_or(true) {
            best = Some((p, n));
        }
        p += 2;
    }
    best.map(|(p, _)| p)
}


/// 老格式（`SougouPY` 开头、4 个 `\0\n` 结尾的头字段、词条区
/// `[u16 拼音字符数][拼音][u16 词字符数][词]`）。
///
/// 保留它是因为早期导出工具产出的 `.scel` 是这个格式，而现在的搜狗客户端
/// 已经不写这种了（见 `parse_scel_modern`）。
fn parse_scel_legacy(data: &[u8]) -> Vec<String> {
    let mut base = 9usize;
    let mut heads = vec![base];
    for _ in 0..4 {
        while base + 1 < data.len() && data[base..base + 2] != [0x0a, 0x00] {
            base += 2;
        }
        base += 2;
        heads.push(base);
    }

    let parse = |start: usize| -> Vec<String> {
        let mut out = Vec::new();
        let mut p = start;
        while p + 4 <= data.len() {
            let done = (|| {
                let plen = u16_at(data, p)? as usize;
                if !(2..=64).contains(&plen) {
                    return None;
                }
                let py_end = p + 2 + plen * 2;
                if py_end + 2 > data.len() {
                    return None;
                }
                let wlen = u16_at(data, py_end)? as usize;
                if !(2..=16).contains(&wlen) {
                    return None;
                }
                let w_end = py_end + 2 + wlen * 2;
                if w_end > data.len() {
                    return None;
                }
                let w = utf16_at(data, py_end + 2, wlen * 2)?;
                if !is_all_cjk(&w) {
                    return None;
                }
                Some((w, w_end))
            })();
            match done {
                Some((w, next)) => {
                    out.push(w);
                    p = next;
                }
                None => p += 2,
            }
        }
        out
    };

    let mut best = Vec::new();
    for start in heads {
        let got = parse(start);
        if got.len() > best.len() {
            best = got;
        }
        if best.len() > 16 {
            break;
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
// 入口
// ─────────────────────────────────────────────────────────────

/// `Job` 的便捷构造：从扩展名判 [Kind]。
///
/// 手动挑的 `.bin` 一律按宽松解析（`Job.kind = BinLoose`）——手动挑没有「这是
/// 打字历史」的上下文，宁可少收几条，不要因为校验和把整包拒绝掉。
pub fn kind_for_ext(ext: &str) -> Option<Kind> {
    match ext.to_ascii_lowercase().as_str() {
        "scel" => Some(Kind::Scel),
        "bin" => Some(Kind::BinLoose),
        "txt" => Some(Kind::Text),
        _ => None,
    }
}

/// 一批文件 → `词<TAB>全拼` 文本（按 优先级/次数/词短 排好序）。
///
/// 返回 `(文本, 统计)`。文本可以直接喂给 `dict::merge_text`（它负责去重、
/// 追加到 `user_dict.txt`、并编译 `user.bin`）。
///
/// **不碰文件系统**：调用方自己读盘、自己写盘。这样 Windows 壳能在导入前后
/// 打日志、Android 壳能把结果塞进临时文件，都不用改这里。
pub fn encode_jobs(jobs: &[Job], max_len: usize) -> (String, Imported) {
    let mut tally: HashMap<String, (u8, u32)> = HashMap::new();
    let mut files = 0usize;
    let mut scanned = 0usize;
    for job in jobs {
        let words = match job.kind {
            Kind::BinStrict => extract_bin_strict(&job.data),
            Kind::BinLoose => extract_bin_loose(&job.data),
            Kind::Scel => extract_scel(&job.data),
            Kind::Text => extract_txt(&decode_text(&job.data)),
        };
        files += 1;
        scanned += words.len();
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

    let stat = Imported {
        files,
        scanned,
        encoded: rows.len(),
        added: 0, // 由调用方 merge 之后填
        skipped: enc.skipped,
        from_base: enc.from_base,
        from_char: enc.from_char,
    };
    (text, stat)
}

/// 一个文件 → 那份文本。等于 `encode_jobs(&[job], max_len)`，只是省掉拼数组。
pub fn encode_one(job: &Job, max_len: usize) -> (String, Imported) {
    encode_jobs(std::slice::from_ref(job), max_len)
}

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

    /// 一条词都解析不出来时，摘要必须说「没能解析」，不能回「导入成功」。
    ///
    /// 现场（2026-11-06）：Android 上导入一个文件后界面什么都没显示，用户无从判断
    /// 一条词都解析不出来时，摘要必须说「没能解析」，不能回「导入成功」。
    ///
    /// 现场（2026-11-06）：Android 上导入一个文件后界面什么都没显示，用户无从判断
    /// 是导入失败、文件没选上、还是导入了零条。这个测试锁住那个分支。
    /// **真实样本**：`D:\down\中国风景名胜.scel`（53 KB，搜狗现在的导出格式）。
    ///
    /// 之前这个文件解析出 0 条——它既不是老格式（没有 `SougouPY` magic），
    /// 也不符合我们原来猜的词条布局。2026-11-07 按字节对着解出来之后钉了这个测试：
    /// 结构一变，这里立刻红。
    #[test]
    fn real_scel_sample_parses() {
        let path = std::path::Path::new("D:/down/中国风景名胜.scel");
        if !path.exists() {
            return; // 别的机器上没有这个样本，跳过
        }
        let data = std::fs::read(path).unwrap();
        let words = parse_scel_modern(&data).unwrap_or_default();
        assert!(words.len() > 500, "解析出 {} 条，比样本应有的少多了", words.len());
        // 抽查几条样本里确实有的词（名录类词库，前后都有）
        for want in ["天安门", "阿育王塔", "安济桥", "醉白池"] {
            assert!(words.iter().any(|w| w == want), "样本里应该有「{want}」");
        }
        assert!(words.iter().all(|w| w.chars().count() >= 2), "不该有单字词");
        // 音节表要解对，否则注音全是错的
        let (text, stat) = encode_one(
            &Job { label: "t.scel".into(), data, kind: Kind::Scel, prio: 1 },
            MAX_LEN,
        );
        assert!(stat.encoded > 100, "能注音 {} 条", stat.encoded);
        // 抽查一条：解析出来的词必须注得出音，而且拼音是「小写字母 + 空格」
        let line = text.lines().find(|l| l.starts_with("安济桥")).unwrap_or("");
        assert!(!line.is_empty(), "安济桥 应该注得出音");
        let py = line.split_once('\t').map(|(_, p)| p).unwrap_or("");
        assert!(
            !py.is_empty() && py.chars().all(|c| c.is_ascii_lowercase() || c == ' '),
            "拼音格式不对：{line:?}"
        );
    }

    #[test]
    fn empty_result_summary_says_why() {
        let job = Job {
            label: "empty.txt".into(),
            data: b"# nothing here\n\n".to_vec(),
            kind: Kind::Text,
            prio: 1,
        };
        let (text, stat) = encode_one(&job, MAX_LEN);
        assert!(text.is_empty());
        assert_eq!(stat.encoded, 0);
        let s = stat.summary();
        assert!(s.contains("没能"), "{s}");
        assert!(!s.contains("新增"), "零条时不能只说成功：{s}");
    }

    /// 正常文件：文本要能直接喂给 `dict::merge_text`，一行一条「词 + Tab + 全拼」。
    #[test]
    fn encode_one_produces_tab_rows() {
        let job = Job {
            label: "words.txt".into(),
            data: "你好\n世界\n".as_bytes().to_vec(),
            kind: Kind::Text,
            prio: 1,
        };
        let (text, stat) = encode_one(&job, MAX_LEN);
        assert_eq!(stat.files, 1);
        assert_eq!(stat.scanned, 2);
        assert!(stat.encoded > 0, "这两个词都在基础词库里，必须注得出音");
        for line in text.lines() {
            let (w, py) = line.split_once('\t').unwrap_or((line, ""));
            assert!(!w.is_empty() && !py.is_empty(), "每行都得是「词+Tab+全拼」：{line}");
        }
    }

    #[test]
    fn kind_for_ext_maps_by_extension() {
        assert_eq!(kind_for_ext("scel"), Some(Kind::Scel));
        assert_eq!(kind_for_ext(".SCEL"), None, "调用方给的是纯扩展名，不带点");
        assert_eq!(kind_for_ext("SCEL"), Some(Kind::Scel), "大小写不敏感");
        assert_eq!(kind_for_ext("txt"), Some(Kind::Text));
        assert_eq!(kind_for_ext("BIN"), Some(Kind::BinLoose));
        assert_eq!(kind_for_ext("exe"), None, "不认识的扩展名要拒绝而不是瞎猜");
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
