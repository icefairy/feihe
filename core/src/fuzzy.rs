//! 模糊音：把用户敲的码展开成「同音异写」的几个码，让 `n/l`、`zh/z` 这类口音也打得出来。
//!
//! ## 为什么在**码**上做，而不是在全拼上做
//!
//! 双拼每个音节恰好两键，声母和韵母各占一位。所以一条模糊音规则就是**某个位置上的
//! 一次换键**：小鹤里 `z=zh` 就是声母位 `z ↔ v`（`v` 是 `zh` 键），`an=ang` 就是
//! 韵母位 `j ↔ h`。按字母替换（`zh` → `z`）是错的——`zh` 在码里根本不是一个字符。
//!
//! 反过来的做法（先把两键解码成全拼、套规则、再编码）要先解决「`go` 到底是 `guo`
//! 还是 `g`+`o`」这类歧义，得带一份完整的音节表才能判。换键法不需要：位置已经告诉
//! 你哪一位是声母、哪一位是韵母。
//!
//! ## 两类位置，规则不能串用
//!
//! 声母规则（`z=zh`、`n=l`…）**只能用在声母位**：`l` 和 `r` 同时也是韵母键
//! （`l`=`iang/uang`、`r`=`uan/er`），跑错位置会把 `xian` 变成 `xiar` 这种鬼东西。
//! 韵母规则同理只用在韵母位——`f=h` 用到韵母位上，`en` 就变成了 `eh`。
//!
//! ## 零声母是小鹤的特例
//!
//! 小鹤的零声母码不是「声母位 + 韵母键」，而是 `an` → `an`（双字母保留全拼）、
//! `ang` → `ah`（三字母用韵母键）。所以 `an=ang` 在零声母上换的是 `n ↔ h`，
//! 跟有声母时的 `j ↔ h` 不是一对。微软双拼没这个问题（零声母一律 `o` + 韵母键）。
//!
//! ## 排在前面的永远是原样
//!
//! 展开结果的第一项一定等于输入原样，其余按「改了几个位置」由少到多排。`dict::search`
//! 据此把模糊命中的候选排在同码精确命中之后（见 `fuzzy_penalty`），不会出现
//! 「打开模糊音，常用字被口音写法挤下去」。

use crate::shuangpin::{self, Scheme};

/// 展开后最多几个码。多音节长输入理论上是笛卡尔积（3 个位置各 2 种就是 8 个），
/// 再长就没有性价比了——每次展开都要多查一遍词库。
pub const MAX_VARIANTS: usize = 8;

/// 一条模糊音规则。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Rules {
    pub z_zh: bool,
    pub c_ch: bool,
    pub s_sh: bool,
    pub n_l: bool,
    pub f_h: bool,
    pub l_r: bool,
    pub an_ang: bool,
    pub en_eng: bool,
    pub in_ing: bool,
}

/// 配置里用的规则名，顺序就是设置窗的排布顺序。
pub const NAMES: [&str; 9] = [
    "z_zh", "c_ch", "s_sh", "n_l", "f_h", "l_r", "an_ang", "en_eng", "in_ing",
];

impl Rules {
    /// 全关（默认）。
    pub const NONE: Self = Self {
        z_zh: false,
        c_ch: false,
        s_sh: false,
        n_l: false,
        f_h: false,
        l_r: false,
        an_ang: false,
        en_eng: false,
        in_ing: false,
    };

    /// 全开：只有测试用（设置窗是一个一个勾，没有「一键全开」这个按钮）。
    /// 标 `cfg(test)` 是因为发布构建里它一条都进不去——留着会报 dead_code。
    #[cfg(test)]
    pub const ALL: Self = Self {
        z_zh: true,
        c_ch: true,
        s_sh: true,
        n_l: true,
        f_h: true,
        l_r: true,
        an_ang: true,
        en_eng: true,
        in_ing: true,
    };

    pub fn any(&self) -> bool {
        *self != Self::default()
    }

    /// `z_zh` → 给人看的 `z = zh`。
    pub fn label(name: &str) -> String {
        name.replace('_', " = ")
    }

    /// 某条规则开着没有（按 `NAMES` 里的名字查）。
    pub fn is_on(&self, name: &str) -> bool {
        self.flag(name)
    }

    /// 开关某条规则；名字不认识返回 `false`（设置窗按 `NAMES` 派发，不该发生）。
    pub fn set(&mut self, name: &str, on: bool) -> bool {
        self.set_flag(name, on)
    }

    fn flag(&self, name: &str) -> bool {
        match name {
            "z_zh" => self.z_zh,
            "c_ch" => self.c_ch,
            "s_sh" => self.s_sh,
            "n_l" => self.n_l,
            "f_h" => self.f_h,
            "l_r" => self.l_r,
            "an_ang" => self.an_ang,
            "en_eng" => self.en_eng,
            "in_ing" => self.in_ing,
            _ => false,
        }
    }

    fn set_flag(&mut self, name: &str, on: bool) -> bool {
        let slot = match name {
            "z_zh" => &mut self.z_zh,
            "c_ch" => &mut self.c_ch,
            "s_sh" => &mut self.s_sh,
            "n_l" => &mut self.n_l,
            "f_h" => &mut self.f_h,
            "l_r" => &mut self.l_r,
            "an_ang" => &mut self.an_ang,
            "en_eng" => &mut self.en_eng,
            "in_ing" => &mut self.in_ing,
            _ => return false,
        };
        *slot = on;
        true
    }

    /// 解析配置值：`z_zh,n_l` / `z-zh n_l` 都认；空串 = 全关。
    pub fn from_key(s: &str) -> Self {
        let mut rules = Self::default();
        for part in s.split(['|', ',', ' ', ';']).map(str::trim) {
            if part.is_empty() {
                continue;
            }
            let normalized = part.replace('-', "_").to_ascii_lowercase();
            rules.set_flag(&normalized, true);
        }
        rules
    }

    /// 写成配置值，未开启的规则不出现（全关就是空串）。
    pub fn to_key(&self) -> String {
        let mut on: Vec<&str> = Vec::new();
        for name in NAMES {
            if self.flag(name) {
                on.push(name);
            }
        }
        on.join(",")
    }
}

/// 声母位的一对键。两个方案都是 `zh→v`、`ch→i`、`sh→u`，所以这张表不分方案。
fn initial_pair(rules: Rules, name: &str) -> Option<(char, char)> {
    match name {
        "z_zh" if rules.z_zh => Some(('z', 'v')),
        "c_ch" if rules.c_ch => Some(('c', 'i')),
        "s_sh" if rules.s_sh => Some(('s', 'u')),
        "n_l" if rules.n_l => Some(('n', 'l')),
        "f_h" if rules.f_h => Some(('f', 'h')),
        "l_r" if rules.l_r => Some(('l', 'r')),
        _ => None,
    }
}

/// 韵母位的一对键。`zero` 是「小鹤的零声母音节」——那里的第二位不是韵母键，
/// 而是双字母韵母的末字母，所以 `an=ang` 落到 `n ↔ h` 上。
fn final_pair(rules: Rules, scheme: Scheme, name: &str, zero: bool) -> Option<(char, char)> {
    match (name, zero) {
        ("an_ang", false) if rules.an_ang => Some(('j', 'h')),
        ("en_eng", false) if rules.en_eng => Some(('f', 'g')),
        // 小鹤零声母：an = "an"、ang = "ah"；en = "en"、eng = "eg"
        ("an_ang", true) if rules.an_ang && scheme == Scheme::Xiaohe => Some(('n', 'h')),
        ("en_eng", true) if rules.en_eng && scheme == Scheme::Xiaohe => Some(('n', 'g')),
        // 微软零声母是 o + 韵母键，走上面那条非 zero 的路。ing 落在分号上。
        ("in_ing", false) if rules.in_ing => Some(match scheme {
            Scheme::Xiaohe => ('b', 'k'),
            Scheme::Ms => ('n', ';'),
        }),
        // 零声母没有 in（普通话里以 i 开头的音节都写成 y-），这条在 zero 下不成立
        _ => None,
    }
}

/// 一对键「换一次」能变出的写法（不含自己）。
///
/// 只换一处：声母位换声母、韵母位换韵母。两者不交叉，否则 `n=l` 会把
/// `ni`（`ni`）变成 `li`（`li`）之外还顺带动韵母。
fn step(rules: Rules, scheme: Scheme, a: char, b: char) -> Vec<(char, char)> {
    let mut out = Vec::new();
    // 小鹤零声母：第一位是「零声母记号」，不是声母，别动它
    let zero = scheme == Scheme::Xiaohe && matches!(a, 'a' | 'e' | 'o');
    if !zero {
        for name in ["z_zh", "c_ch", "s_sh", "n_l", "f_h", "l_r"] {
            let Some((p, q)) = initial_pair(rules, name) else {
                continue;
            };
            if a == p {
                out.push((q, b));
            } else if a == q {
                out.push((p, b));
            }
        }
    }
    for name in ["an_ang", "en_eng", "in_ing"] {
        let Some((p, q)) = final_pair(rules, scheme, name, zero) else {
            continue;
        };
        if b == p {
            out.push((a, q));
        } else if b == q {
            out.push((a, p));
        }
    }
    out.retain(|&(x, y)| (x, y) != (a, b) && shuangpin::is_valid_pair(scheme, x, y));
    out
}

/// 一个音节位置的全部写法，第一个是原样。
///
/// 规则之间会叠加（开了 `n=l` 和 `l=r`，`n` 能一路走到 `r`），所以要跑到不再增长；
/// 规则集很小，两轮就收敛。
fn pair_alternatives(rules: Rules, scheme: Scheme, a: char, b: char) -> Vec<(char, char)> {
    let mut forms = vec![(a, b)];
    let mut i = 0;
    while i < forms.len() {
        let (x, y) = forms[i];
        for next in step(rules, scheme, x, y) {
            if !forms.contains(&next) {
                forms.push(next);
            }
        }
        i += 1;
    }
    forms
}

/// 展开一个码。返回值第一项恒等于 `code`（调用方据此区分「原样命中」和「模糊命中」）。
///
/// 非纯小写（含手动分隔符 `'`、`;`、大写）一律不展开：分隔符会打乱「两键一组」的
/// 对齐，硬按位置换会换到错误的地方。
pub fn expand(rules: Rules, scheme: Scheme, code: &str) -> Vec<String> {
    let original = code.to_string();
    if !rules.any()
        || code.is_empty()
        || !code.bytes().all(|b| b.is_ascii_lowercase())
    {
        return vec![original];
    }
    let bytes = code.as_bytes();
    let pairs = bytes.len() / 2;
    let tail = if bytes.len() % 2 == 1 {
        Some(bytes[bytes.len() - 1] as char)
    } else {
        None
    };

    // 逐位做笛卡尔积，代价 = 这个位置改过（不是原样）的个数
    let mut acc: Vec<(String, usize)> = vec![(String::new(), 0)];
    for i in 0..pairs {
        let a = bytes[i * 2] as char;
        let b = bytes[i * 2 + 1] as char;
        let alts = pair_alternatives(rules, scheme, a, b);
        let mut next: Vec<(String, usize)> = Vec::with_capacity(acc.len() * alts.len());
        for (head, cost) in &acc {
            for (k, &(x, y)) in alts.iter().enumerate() {
                let mut s = String::with_capacity(head.len() + 2);
                s.push_str(head);
                s.push(x);
                s.push(y);
                next.push((s, cost + usize::from(k != 0)));
            }
        }
        acc = next;
        // 长输入的组合数会翻着涨，留最省的若干条就够（最后还要再截一次）
        if acc.len() > MAX_VARIANTS * 8 {
            acc.sort_by_key(|x| x.1);
            acc.truncate(MAX_VARIANTS * 8);
        }
    }

    // 尾巴：只敲了半个音节（双拼里就是一个声母键），只能套声母规则
    if let Some(t) = tail {
        let mut tails = vec![t];
        for name in ["z_zh", "c_ch", "s_sh", "n_l", "f_h", "l_r"] {
            let Some((p, q)) = initial_pair(rules, name) else {
                continue;
            };
            let other = if t == p {
                q
            } else if t == q {
                p
            } else {
                continue;
            };
            if shuangpin::is_initial_key(scheme, other) && !tails.contains(&other) {
                tails.push(other);
            }
        }
        let mut next: Vec<(String, usize)> = Vec::with_capacity(acc.len() * tails.len());
        for (head, cost) in &acc {
            for (k, &c) in tails.iter().enumerate() {
                let mut s = String::with_capacity(head.len() + 1);
                s.push_str(head);
                s.push(c);
                next.push((s, cost + usize::from(k != 0)));
            }
        }
        acc = next;
    }

    // 稳定按代价排序（`sort_by_key` 是稳定排序），原样那一条自然在最前
    acc.sort_by_key(|x| x.1);
    let mut out: Vec<String> = Vec::with_capacity(MAX_VARIANTS.min(acc.len()));
    for (code, _) in acc {
        if code != original && out.contains(&code) {
            continue;
        }
        out.push(code);
        if out.len() >= MAX_VARIANTS {
            break;
        }
    }
    if out.first().map(String::as_str) != Some(original.as_str()) {
        out.insert(0, original);
        out.truncate(MAX_VARIANTS);
    }
    out
}

/// 模糊命中的候选在学习计数上要减掉这么多，才允许排到同码精确命中的前面。
///
/// 为什么不是硬性「模糊永远排在后面」：`n=l` 的人打 `lan` 就是想打「南」，他选过十几次
/// 之后「南」理应在前。减 3 的意思是「至少要选过 4 次才翻得过一个原样命中」——
/// 一次误开模糊音不会立刻打乱手感。
pub const COUNT_PENALTY: u32 = 3;

#[cfg(test)]
mod tests {
    use super::*;

    fn xh(rules: Rules, code: &str) -> Vec<String> {
        expand(rules, Scheme::Xiaohe, code)
    }

    /// 小鹤的 `z=zh`：声母位 `z` ↔ `v`，韵母位不动。
    #[test]
    fn z_zh_swaps_the_initial_key_only() {
        let rules = Rules {
            z_zh: true,
            ..Default::default()
        };
        // 自 zi = z + i；知 zhi = v + i
        assert_eq!(xh(rules, "zi"), ["zi", "vi"]);
        assert_eq!(xh(rules, "vi"), ["vi", "zi"]);
        // 韵母位的 i（chi = ii）不该被动
        assert_eq!(xh(rules, "ii"), ["ii"]);
    }

    /// 韵母规则只动韵母位：`an=ang` 是小鹤的 `j ↔ h`。
    #[test]
    fn an_ang_swaps_the_final_key_only() {
        let rules = Rules {
            an_ang: true,
            ..Default::default()
        };
        // 三 san = s + j；桑 sang = s + h
        assert_eq!(xh(rules, "sj"), ["sj", "sh"]);
        // 声母位不为所动（jiang 的 j 是声母）
        assert!(xh(rules, "jl").contains(&"jl".to_string()));
    }

    /// 声母规则不能跑到韵母位上去：`l`/`r`/`n` 同时是韵母键。
    #[test]
    fn initial_rules_never_touch_the_final_slot() {
        let rules = Rules {
            n_l: true,
            l_r: true,
            ..Default::default()
        };
        // liang = l + l（第一位声母 l、第二位韵母 l）
        // 只有声母位能换：liang → niang（n l）/ riang 不存在
        let got = xh(rules, "ll");
        assert!(got.contains(&"ll".to_string()));
        assert!(got.contains(&"nl".to_string()), "{got:?}");
        assert!(!got.iter().any(|c| c.ends_with('r')), "韵母位不该被换：{got:?}");
    }

    /// 规则会叠加：`n=l` + `l=r` 让 `n` 一路走到 `r`。
    #[test]
    fn rules_chain() {
        let rules = Rules {
            n_l: true,
            l_r: true,
            ..Default::default()
        };
        let got = xh(rules, "ni");
        assert!(got.contains(&"ni".to_string()));
        assert!(got.contains(&"li".to_string()));
        assert!(got.contains(&"ri".to_string()), "{got:?}");
    }

    /// 换出来的码必须是结构合法的：`en` 套 `an=ang` 不能变成 `eh`。
    #[test]
    fn illegal_keys_are_dropped() {
        let rules = Rules {
            an_ang: true,
            ..Default::default()
        };
        // 零声母 en = "en"，换 n ↔ h 得 "eh"——不是合法码，丢掉
        assert_eq!(xh(rules, "en"), ["en"]);
        // 零声母 an = "an" → ang = "ah"，这个是合法换法
        assert_eq!(xh(rules, "an"), ["an", "ah"]);
    }

    /// 打了一半的尾巴只套声母规则。
    #[test]
    fn trailing_half_syllable_only_swaps_initials() {
        let rules = Rules {
            z_zh: true,
            n_l: true,
            ..Default::default()
        };
        // "z" 是半个音节（下一个字的声母）
        let got = xh(rules, "z");
        assert!(got.contains(&"z".to_string()));
        assert!(got.contains(&"v".to_string()), "{got:?}");
    }

    /// 结果永远以原样开头，且不超过上限。
    #[test]
    fn original_always_first_and_bounded() {
        let rules = Rules::ALL;
        for code in ["zi", "sj", "ll", "vhsjli", "z", "nihaoma"] {
            let got = xh(rules, code);
            assert_eq!(got.first().map(String::as_str), Some(code), "{code} → {got:?}");
            assert!(got.len() <= MAX_VARIANTS, "{code} → {got:?}");
        }
    }

    /// 全关就是原样一条；带分隔符的码不展开。
    #[test]
    fn off_or_separated_codes_stay_untouched() {
        assert_eq!(xh(Rules::default(), "zi"), ["zi"]);
        assert_eq!(xh(Rules::ALL, "ni'hao"), ["ni'hao"]);
        assert_eq!(xh(Rules::ALL, ""), [""]);
    }

    /// 配置值往返：只写开着的规则，解析认得 `-` 和 `_`。
    #[test]
    fn config_key_round_trips() {
        let mut rules = Rules::default();
        assert_eq!(rules.to_key(), "");
        rules.z_zh = true;
        rules.n_l = true;
        assert_eq!(rules.to_key(), "z_zh,n_l");
        assert_eq!(Rules::from_key("z_zh,n_l"), rules);
        assert_eq!(Rules::from_key("z-zh n_l"), rules);
        assert_eq!(Rules::from_key(""), Rules::default());
        assert_eq!(Rules::from_key("乱写"), Rules::default());
    }

    /// 微软双拼：`ing` 在分号上，所以 `in=ing` 换的是 `n ↔ ;`。
    #[test]
    fn ms_in_ing_uses_the_semicolon_key() {
        let rules = Rules {
            in_ing: true,
            ..Default::default()
        };
        // 金 jin = j + n；京 jing = j + ;
        let got = expand(rules, Scheme::Ms, "jn");
        assert_eq!(got, ["jn", "j;"]);
    }
}
