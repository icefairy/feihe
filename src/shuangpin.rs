//! 双拼方案：全拼音节 → 两键码
//!
//! 支持两套方案：小鹤双拼、微软双拼。词库里存的是**全拼**，运行时按当前方案
//! 现场编码——这样切换方案、导入导出词库都不需要重新生成数据文件。
//!
//! ## 小鹤双拼
//! 声母 `zh→v`、`ch→i`、`sh→u`；零声母用「韵母首字母 + 韵母键」（`an`→`aj`）。
//!
//! ## 微软双拼
//! 声母同样 `zh→v`、`ch→i`、`sh→u`；零声母用 `o` 占位（`an`→`oj`）；
//! `ing` 落在分号键 `;` 上，`ü` 落在 `y` 上。

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

/// 单个音节 → 两键码（不分配，返回两个字符）。无法编码返回 None。
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
            // 小鹤：首键取韵母首字母（只可能是 a / e / o）
            Scheme::Xiaohe => {
                let first = b[0] as char;
                if !matches!(first, 'a' | 'e' | 'o') {
                    return None;
                }
                Some((first, key))
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
        assert_eq!(xh("a"), "aa");
        assert_eq!(xh("ai"), "ad");
        assert_eq!(xh("an"), "aj");
        assert_eq!(xh("ang"), "ah");
        assert_eq!(xh("ao"), "ac");
        assert_eq!(xh("e"), "ee");
        assert_eq!(xh("ei"), "ew");
        assert_eq!(xh("en"), "ef");
        assert_eq!(xh("eng"), "eg");
        assert_eq!(xh("er"), "er");
        assert_eq!(xh("o"), "oo");
        assert_eq!(xh("ou"), "oz");
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

    #[test]
    fn scheme_keys() {
        assert_eq!(Scheme::from_key("xiaohe"), Scheme::Xiaohe);
        assert_eq!(Scheme::from_key("ms"), Scheme::Ms);
        assert_eq!(Scheme::from_key("garbage"), Scheme::Xiaohe);
        assert!(Scheme::Ms.is_code_char(';'));
        assert!(!Scheme::Xiaohe.is_code_char(';'));
    }
}
