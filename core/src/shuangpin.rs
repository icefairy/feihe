//! 双拼方案：全拼音节 → 两键码
//!
//! 支持两套方案：小鹤双拼、微软双拼。词库里存的是**全拼**，运行时按当前方案
//! 现场编码——这样切换方案、导入导出词库都不需要重新生成数据文件。
//!
//! ## 小鹤双拼
//! 声母 `zh→v`、`ch→i`、`sh→u`；零声母按韵母长度分三种：
//! 单字母重复（`a`→`aa`）、双字母保持全拼（`an`→`an`、`ai`→`ai`）、
//! 三字母用韵母键（`ang`→`ah`）。
//!
//! ## 微软双拼
//! 声母同样 `zh→v`、`ch→i`、`sh→u`；零声母一律用 `o` 占位（`an`→`oj`）；
//! `ing` 落在分号键 `;` 上，`ü` 落在 `y` 上。

use std::collections::HashSet;
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scheme {
    Xiaohe,
    Ms,
}

impl Scheme {
    pub fn from_key(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "ms" | "microsoft" | "微软" => Scheme::Ms,
            _ => Scheme::Xiaohe,
        }
    }

    pub fn key(&self) -> &'static str {
        match self {
            Scheme::Xiaohe => "xiaohe",
            Scheme::Ms => "ms",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Scheme::Xiaohe => "小鹤双拼",
            Scheme::Ms => "微软双拼",
        }
    }

    /// 该方案里哪些字符是合法的编码键（微软双拼的 `ing` 要用分号）
    pub fn is_code_char(&self, c: char) -> bool {
        c.is_ascii_lowercase() || (matches!(self, Scheme::Ms) && c == ';')
    }

    fn finals(&self) -> &'static [(&'static str, char)] {
        match self {
            Scheme::Xiaohe => XIAOHE_FINALS,
            Scheme::Ms => MS_FINALS,
        }
    }
}

const INITIALS: &[u8] = b"bpmfdtnlgkhjqxrzcsyw";

/// 小鹤韵母键位
const XIAOHE_FINALS: &[(&str, char)] = &[
    ("a", 'a'),
    ("o", 'o'),
    ("e", 'e'),
    ("i", 'i'),
    ("u", 'u'),
    ("v", 'v'),
    ("ai", 'd'),
    ("ei", 'w'),
    ("ao", 'c'),
    ("ou", 'z'),
    ("an", 'j'),
    ("en", 'f'),
    ("ang", 'h'),
    ("eng", 'g'),
    ("ong", 's'),
    ("ia", 'x'),
    ("ie", 'p'),
    ("iao", 'n'),
    ("iu", 'q'),
    ("ian", 'm'),
    ("in", 'b'),
    ("iang", 'l'),
    ("ing", 'k'),
    ("iong", 's'),
    ("ua", 'x'),
    ("uo", 'o'),
    ("uai", 'k'),
    ("ui", 'v'),
    ("uan", 'r'),
    ("un", 'y'),
    ("uang", 'l'),
    ("ue", 't'),
    ("ve", 't'),
    ("van", 'r'),
    ("vn", 'y'),
    ("er", 'r'),
];

/// 微软韵母键位。`ing` 在分号键，`ü`(v) 和 `uai` 共用 y，`ün`(vn) 和 `un` 共用 p。
const MS_FINALS: &[(&str, char)] = &[
    ("a", 'a'),
    ("o", 'o'),
    ("e", 'e'),
    ("i", 'i'),
    ("u", 'u'),
    ("v", 'y'),
    ("ai", 'l'),
    ("ei", 'z'),
    ("ao", 'k'),
    ("ou", 'b'),
    ("an", 'j'),
    ("en", 'f'),
    ("ang", 'h'),
    ("eng", 'g'),
    ("ong", 's'),
    ("ia", 'w'),
    ("ie", 'x'),
    ("iao", 'c'),
    ("iu", 'q'),
    ("ian", 'm'),
    ("in", 'n'),
    ("iang", 'd'),
    ("ing", ';'),
    ("iong", 's'),
    ("ua", 'w'),
    ("uo", 'o'),
    ("uai", 'y'),
    ("ui", 'v'),
    ("uan", 'r'),
    ("un", 'p'),
    ("uang", 'd'),
    ("ue", 't'),
    ("ve", 't'),
    ("van", 'r'),
    ("vn", 'p'),
    ("er", 'r'),
];

fn final_key(scheme: Scheme, f: &str) -> Option<char> {
    scheme
        .finals()
        .iter()
        .find(|(k, _)| *k == f)
        .map(|(_, c)| *c)
}

/// 声母键：20 个单字母声母 + `zh/ch/sh` 的三个代键。
///
/// `v/i/u` 同时是韵母键（`v`=`ui/ü`、`i`=`i`、`u`=`u`），但**在声母位**它们只表示
/// `zh/ch/sh`——双拼每个音节恰好两键，位置本身就是语义。
const INITIAL_KEYS: &[char] = &[
    'b', 'p', 'm', 'f', 'd', 't', 'n', 'l', 'g', 'k', 'h', 'j', 'q', 'x', 'r', 'z', 'c', 's', 'y',
    'w', 'v', 'i', 'u',
];

/// 结构上可能出现的两键码集合（`(声母位, 韵母位)`）。
///
/// 为什么需要它：模糊音是把某一位置的键换成「同音的另一套写法」（`z`↔`v`、`n`↔`l`），
/// 换完必须确认结果还真是个合法码，否则会拿 `eh` 这种不存在的码去查库，白跑一趟
/// 还可能让候选窗闪空。
///
/// 构造方式刻意**宽松**：把每个声母键配上每个韵母键，加零声母的合法组合。像
/// `b`+`uang` 这种普通话里没有的音节也会进集合，但它的码本来就等于某个真实组合的码
/// （`buang` 和 `bang` 在双拼里不同码，但 `b`+`uang` 生成的 `bl` 这种码没人查得到东西），
/// 多收一个只会多一次空查询，不会给出错候选。反过来漏收会直接让模糊音失效，那更糟。
fn valid_pairs(scheme: Scheme) -> &'static HashSet<(char, char)> {
    static XH: OnceLock<HashSet<(char, char)>> = OnceLock::new();
    static MS: OnceLock<HashSet<(char, char)>> = OnceLock::new();
    let cell = match scheme {
        Scheme::Xiaohe => &XH,
        Scheme::Ms => &MS,
    };
    cell.get_or_init(|| {
        let mut set = HashSet::with_capacity(2048);
        for &ini in INITIAL_KEYS {
            for (_, key) in scheme.finals() {
                set.insert((ini, *key));
            }
        }
        // 零声母：直接按「韵母本身」编码，落到哪个两键就收哪个
        for (fin, _) in scheme.finals() {
            if let Some(pair) = encode_chars(scheme, fin) {
                set.insert(pair);
            }
        }
        set
    })
}

/// 这两键是不是结构上可能存在的双拼码。
pub fn is_valid_pair(scheme: Scheme, a: char, b: char) -> bool {
    valid_pairs(scheme).contains(&(a, b))
}

/// 这个键能不能坐在声母位上（给「只敲了半截音节」的尾巴用）。
pub fn is_initial_key(scheme: Scheme, c: char) -> bool {
    valid_pairs(scheme).iter().any(|(a, _)| *a == c)
}/// 单个音节 → 两键码（不分配，返回两个字符）。无法编码返回 None。
fn encode_chars(scheme: Scheme, syl: &str) -> Option<(char, char)> {
    let s = syl.trim();
    if s.is_empty() || !s.is_ascii() {
        return None;
    }
    // 数据里本来就是小写，只有真出现大写时才走复制路径
    let lowered;
    let s: &str = if s.bytes().any(|b| b.is_ascii_uppercase()) {
        lowered = s.to_ascii_lowercase();
        &lowered
    } else {
        s
    };
    let b = s.as_bytes();

    // 声母：zh/ch/sh 映射到 v/i/u，单字母声母取自身，其余视为零声母
    let (initial, fin) = if b.len() >= 3 && matches!(&s[..2], "zh" | "ch" | "sh") {
        let ini = match &s[..2] {
            "zh" => 'v',
            "ch" => 'i',
            _ => 'u', // sh
        };
        (Some(ini), &s[2..])
    } else if INITIALS.contains(&b[0]) {
        (Some(b[0] as char), &s[1..])
    } else {
        (None, s)
    };

    let key = final_key(scheme, fin)?;
    match initial {
        Some(ini) => Some((ini, key)),
        None => match scheme {
            // 小鹤的零声母按**韵母长度**分三种（官方「小鹤入门」1.1 双拼）：
            //   单字母 a/o/e      → 零声母 + 韵母所在键   啊=aa 哦=oo 额=ee
            //   双字母 ai/ei/ao/ou/an/en/er → 零声母 + 韵母**末**字母，表现同全拼
            //                                爱=ai 恩=en 欧=ou 安=an 儿=er
            //   三字母 ang/eng    → 零声母 + 韵母所在键   昂=ah 鞥=eg
            //
            // 注意中间那条：双字母韵母**不走韵母键**。早期版本一刀切按「首字母 +
            // 韵母键」处理，把 `an` 算成 `aj`，于是「按钮」要打 `ajnq` 而不是
            // `annq`，所有零声母词都错位——这是用户按官方反查工具报上来的。
            Scheme::Xiaohe => {
                let first = b[0] as char;
                // 零声母只存在这 12 个韵母（啊 哦 额 爱 诶 熬 欧 安 恩 昂 鞥 儿）。
                // 光看「首字母是 a/e/o」不够：`ong` 也满足，但它不是独立音节
                // （总是跟声母走：d-ong、zh-ong），放进来会让全拼分词多出假边界。
                const ZERO_INITIAL: [&str; 12] =
                    ["a", "o", "e", "ai", "ei", "ao", "ou", "an", "en", "ang", "eng", "er"];
                if !ZERO_INITIAL.contains(&fin) {
                    return None;
                }
                if fin.len() == 2 {
                    Some((first, fin.as_bytes()[1] as char))
                } else {
                    Some((first, key))
                }
            }
            // 微软：用 o 作虚声母
            Scheme::Ms => Some(('o', key)),
        },
    }
}

/// 单个音节 → 两键码。无法编码（`ng`、`hm` 之类）返回 None。
pub fn encode_syllable(scheme: Scheme, syl: &str) -> Option<String> {
    let (a, b) = encode_chars(scheme, syl)?;
    Some(String::from_iter([a, b]))
}

/// 这个字符串是不是一个结构合法的汉语拼音音节（**不查词库**，只看拼写）。
///
/// 用途：全拼的智能分隔要把 `nihaoshijie` 切成 `ni`/`hao`/`shi`/`jie`，
/// 而音节长度不定（1~6 字符），只能靠「这个子串是不是合法音节」来判边界。
///
/// 实现上直接复用 `encode_chars` 的韵母表：能编出双拼码的就是合法音节。
/// 表里没有的韵母（`ng`、`hm`、`hng` 这类叹词）会被拒掉，这在分词场景下是对的
/// ——它们几乎不会出现在连续输入的串里，但会导致大量假边界。
///
/// 用小鹤表做校验是刻意的：小鹤的零声母规则（必须 a/e/o 开头）正好是普通话
/// 的真实约束，比微软双拼「用 o 占位」更严格，拿来当校验器更合适。
pub fn is_syllable(s: &str) -> bool {
    encode_chars(Scheme::Xiaohe, s).is_some()
}

/// 一个拼音串最多几个字符的音节：`zhuang` / `chuang` / `shuang`。
pub const MAX_SYLLABLE_LEN: usize = 6;

/// 整串拼音（音节间空格分隔）→ 连续码。任一音节编不出来就整体放弃。
pub fn encode_pinyin(scheme: Scheme, pinyin: &str) -> Option<String> {
    let mut out = String::with_capacity(pinyin.len() * 2);
    for syl in pinyin.split_whitespace() {
        let (a, b) = encode_chars(scheme, syl)?;
        out.push(a);
        out.push(b);
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xh(p: &str) -> String {
        encode_pinyin(Scheme::Xiaohe, p).unwrap_or_else(|| panic!("cannot encode {p}"))
    }
    fn ms(p: &str) -> String {
        encode_pinyin(Scheme::Ms, p).unwrap_or_else(|| panic!("cannot encode {p}"))
    }

    #[test]
    fn xiaohe_single() {
        assert_eq!(xh("ni"), "ni");
        assert_eq!(xh("hao"), "hc");
        assert_eq!(xh("zhong"), "vs");
        assert_eq!(xh("guo"), "go");
        assert_eq!(xh("de"), "de");
        assert_eq!(xh("xue"), "xt");
        assert_eq!(xh("lve"), "lt");
        assert_eq!(xh("lv"), "lv");
        assert_eq!(xh("quan"), "qr");
        assert_eq!(xh("jun"), "jy");
        assert_eq!(xh("ying"), "yk");
        assert_eq!(xh("kuai"), "kk");
        assert_eq!(xh("liang"), "ll");
        assert_eq!(xh("zou"), "zz");
        assert_eq!(xh("xia"), "xx");
    }

    #[test]
    fn xiaohe_retroflex() {
        // 小鹤：zh->v、ch->i、sh->u
        assert_eq!(xh("zha"), "va");
        assert_eq!(xh("cha"), "ia");
        assert_eq!(xh("sha"), "ua");
        assert_eq!(xh("zhang"), "vh");
        assert_eq!(xh("chang"), "ih");
        assert_eq!(xh("shang"), "uh");
        assert_eq!(xh("shi"), "ui");
        assert_eq!(xh("zhi"), "vi");
        assert_eq!(xh("chi"), "ii");
        assert_eq!(xh("shuo"), "uo");
        assert_eq!(xh("shui"), "uv");
        assert_eq!(xh("chuan"), "ir");
        assert_eq!(xh("zhuang"), "vl");
        assert_eq!(xh("zhui"), "vv");
    }

    #[test]
    fn xiaohe_zero_initial() {
        // 单字母：重复该字母
        assert_eq!(xh("a"), "aa");
        assert_eq!(xh("e"), "ee");
        assert_eq!(xh("o"), "oo");
        // 双字母：零声母 + 韵母末字母 = 全拼形式
        assert_eq!(xh("ai"), "ai");
        assert_eq!(xh("ei"), "ei");
        assert_eq!(xh("ao"), "ao");
        assert_eq!(xh("ou"), "ou");
        assert_eq!(xh("an"), "an");
        assert_eq!(xh("en"), "en");
        assert_eq!(xh("er"), "er");
        // 三字母：零声母 + 韵母所在键
        assert_eq!(xh("ang"), "ah");
        assert_eq!(xh("eng"), "eg");
    }

    /// 用户可见的硬规则：小鹤下「按钮」= `annq`（不是 `ajnq`）。
    /// 这条被官方反查工具 react.xhup.club 验证过，别再改回去。
    #[test]
    fn xiaohe_button_is_annq() {
        assert_eq!(xh("an niu"), "annq");
        assert_eq!(xh("an quan"), "anqr");
        assert_eq!(xh("ai qing"), "aiqk");
        assert_eq!(xh("ou zhou"), "ouvz");
        assert_eq!(xh("ang zang"), "ahzh");
    }

    #[test]
    fn xiaohe_words() {
        assert_eq!(xh("ni hao"), "nihc");
        assert_eq!(xh("zhong guo"), "vsgo");
        assert_eq!(xh("wo men"), "womf");
        assert_eq!(xh("shi jie"), "uijp");
    }

    #[test]
    fn ms_single() {
        assert_eq!(ms("mian"), "mm"); // ian -> m
        assert_eq!(ms("lian"), "lm");
        assert_eq!(ms("dian"), "dm");
        assert_eq!(ms("nao"), "nk"); // ao -> k
        assert_eq!(ms("xie"), "xx"); // ie -> x
        assert_eq!(ms("qiu"), "qq"); // iu -> q
        assert_eq!(ms("yun"), "yp"); // un -> p
        assert_eq!(ms("lou"), "lb"); // ou -> b
        assert_eq!(ms("liang"), "ld"); // iang -> d
        assert_eq!(ms("zhuang"), "vd"); // zh->v, uang->d
        assert_eq!(ms("chuang"), "id");
        assert_eq!(ms("shuang"), "ud");
        assert_eq!(ms("lai"), "ll"); // ai -> l
        assert_eq!(ms("kao"), "kk"); // ao -> k
        assert_eq!(ms("en"), "of"); // 零声母 o 占位
    }

    #[test]
    fn ms_semicolon_ing() {
        assert_eq!(ms("ying"), "y;");
        assert_eq!(ms("jing"), "j;");
        assert_eq!(ms("ping"), "p;");
    }

    #[test]
    fn ms_zero_initial() {
        assert_eq!(ms("ai"), "ol");
        assert_eq!(ms("an"), "oj");
        assert_eq!(ms("ao"), "ok");
        assert_eq!(ms("ou"), "ob");
        assert_eq!(ms("er"), "or");
        assert_eq!(ms("o"), "oo");
    }

    #[test]
    fn ms_umlaut() {
        assert_eq!(ms("nv"), "ny"); // ü -> y
        assert_eq!(ms("lv"), "ly");
        assert_eq!(ms("nve"), "nt"); // üe -> t
        assert_eq!(ms("jun"), "jp"); // ün -> p
        assert_eq!(ms("juan"), "jr");
    }

    #[test]
    fn rejects_unencodable() {
        assert!(encode_syllable(Scheme::Xiaohe, "ng").is_none());
        assert!(encode_syllable(Scheme::Xiaohe, "hm").is_none());
        assert!(encode_syllable(Scheme::Xiaohe, "").is_none());
    }

    /// 零声母只有 12 个韵母。`ong` 看着像「o 开头」，其实不是独立音节
    /// （只有 dong/zhong/cong…），当成音节会让全拼分词切出假边界。
    #[test]
    fn ong_is_not_a_standalone_syllable() {
        assert!(is_syllable("o"));
        assert!(is_syllable("ou"));
        assert!(is_syllable("er"));
        assert!(!is_syllable("ong"), "ong 不是独立音节");
        assert!(!is_syllable("ia"));
        assert!(!is_syllable("uang"));
    }

    #[test]
    fn scheme_keys() {
        assert_eq!(Scheme::from_key("xiaohe"), Scheme::Xiaohe);
        assert_eq!(Scheme::from_key("ms"), Scheme::Ms);
        assert_eq!(Scheme::from_key("garbage"), Scheme::Xiaohe);
        assert!(Scheme::Ms.is_code_char(';'));
        assert!(!Scheme::Xiaohe.is_code_char(';'));
    }
}

/// 全拼串 → 音节序列。
///
/// DP：`ok[i]` 表示 `s[i..]` 能否完整切成合法音节；可行时贪心取最长
/// （`xian` 优先于 `xi`+`an`，符合「长音节优先」的直觉）。
/// 切不出来返回 `None`——说明用户还没敲完，或者敲了不存在的拼写。
///
/// ## 为什么在 core 里
///
/// 这是**纯函数**，只依赖 `is_syllable`。它原来在 `textservice` 里（需要 `&self`
/// 的邻居太多就一直没搬），结果 Android 端要用「全拼串 → 码」就得在 Kotlin 里重写
/// 一遍 —— 那是第二份真值。搬过来之后 Windows 和 Android 用的是同一份。
pub fn split_pinyin(s: &str) -> Option<Vec<String>> {
    // 只接受纯字母。空格/标点必须在这里挡掉：`encode_chars` 会 `trim()`，
    // 于是 `at(i, len)` 取到「 ha」这种带前导空格的片段时会被当成合法音节 `ha`，
    // `ni hao` 就能切出 `ni` + ` ha` + `o`，编出一个看似正常其实错位的码。
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let at = |i: usize, len: usize| -> String { chars[i..i + len].iter().collect() };

    let mut ok = vec![false; n + 1];
    ok[n] = true;
    for i in (0..n).rev() {
        for len in 1..=MAX_SYLLABLE_LEN.min(n - i) {
            if ok[i + len] && is_syllable(&at(i, len)) {
                ok[i] = true;
                break;
            }
        }
    }
    if !ok[0] {
        return None;
    }

    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        let mut step = 0;
        for len in (1..=MAX_SYLLABLE_LEN.min(n - i)).rev() {
            if ok[i + len] && is_syllable(&at(i, len)) {
                step = len;
                break;
            }
        }
        if step == 0 {
            return None;
        }
        out.push(at(i, step));
        i += step;
    }
    Some(out)
}

/// 全拼串 → 双拼码（一个音节气两键）。
///
/// 和 [`encode_pinyin`] 的区别：那个按**空格**切音节（给词库这种规范输入用），
/// 这个自己**猜**音节边界（给键盘上连续敲出来的 `nihao` 用）。
/// `nihao` → `nihc`，`zhongguo` → `vsgo`。
///
/// 切不出合法音节（还没打完、或者拼错）返回 `None`。
pub fn encode_phrase(scheme: Scheme, pinyin: &str) -> Option<String> {
    let syllables = split_pinyin(pinyin)?;
    let mut out = String::with_capacity(syllables.len() * 2);
    for syl in &syllables {
        let (a, b) = encode_chars(scheme, syl)?;
        out.push(a);
        out.push(b);
    }
    Some(out)
}

#[cfg(test)]
mod split_tests {
    use super::*;

    /// 切分的核心承诺：**相邻两个音节拼起来不能有歧义**。
    ///
    /// `xian` 既可以是「先」，也可以是「西 + 安」；这里按最长优先取 `xian`。
    #[test]
    fn splits_prefer_the_longest_syllable() {
        assert_eq!(split_pinyin("xian").unwrap(), vec!["xian"]);
        assert_eq!(
            split_pinyin("xianan").unwrap(),
            vec!["xian", "an"],
            "长音节优先，不是 xi+an+an"
        );
        assert_eq!(split_pinyin("nihao").unwrap(), vec!["ni", "hao"]);
        assert_eq!(split_pinyin("zhongguo").unwrap(), vec!["zhong", "guo"]);
    }

    /// 切不干净就返回 `None`——用户还没打完（`nih`）或者拼了不存在的音节（`zzz`）。
    #[test]
    fn incomplete_input_is_rejected() {
        assert!(split_pinyin("").is_none());
        assert!(split_pinyin("nih").is_none(), "半截音节");
        assert!(split_pinyin("zzz").is_none(), "不存在的拼写");
    }

    /// 键盘上敲出来的是**没有空格**的一串，这正是 `encode_phrase` 存在的理由：
    /// `encode_pinyin` 按空格切，喂它 `nihao` 会当成一个音节而失败。
    #[test]
    fn encode_phrase_takes_unspaced_input() {
        assert_eq!(encode_phrase(Scheme::Xiaohe, "nihao").as_deref(), Some("nihc"));
        assert_eq!(
            encode_phrase(Scheme::Xiaohe, "zhongguo").as_deref(),
            Some("vsgo")
        );
        // 带空格反而应该走不通——那条路是 `encode_pinyin` 的
        assert!(encode_phrase(Scheme::Xiaohe, "ni hao").is_none());
    }
}
