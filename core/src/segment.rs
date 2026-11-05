//! 编码 → 音节 → 分段：长串输入的两级切分。
//!
//! ## 为什么在 core 里
//!
//! 这段逻辑原来长在 `textservice`（Windows 壳）上，是分词态的核心：用户打一长串
//! `vhsjli`，先拆成音节 `vh|sj|li`，再在「每段都查得到候选」的前提下 DP 切成
//! `vhsj|li`（张三|李）。Android 端要做同一件事就必须复用这份——**在 Kotlin 里
//! 重写一遍分段规则是绝对不行的**，那一份和这一份永远会对不上，而症状是「手机上
//! 切出来的词和桌面不一样」，没人查得到原因。
//!
//! 搬出来之后两边共用同一份真值，Windows 侧的函数只剩转发。

use crate::shuangpin;

/// 用户强制指定的音节边界（`vh'sjli` 里的那个撇号）。
///
/// 键盘上不显示这个键，但它是「我说这里断」的唯一手段，桌面端一直支持。
pub const SEP: char = '\'';

/// 双拼码 → 音节序列（每两键一个音节）。
///
/// 某一段长度是奇数就整体返回空：那说明用户还没打完半个音节，**不能猜**。
pub fn double_syllables(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    for group in code.split(SEP) {
        if group.is_empty() {
            continue; // 连续分隔符 / 结尾多敲了一个
        }
        let chars: Vec<char> = group.chars().collect();
        if chars.len() % 2 != 0 {
            return Vec::new();
        }
        for c in chars.chunks(2) {
            out.push(c.iter().collect());
        }
    }
    out
}

/// 全拼串 → 音节序列。任一分组切不出合法音节就整体放弃。
pub fn full_syllables(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    for group in code.split(SEP) {
        if group.is_empty() {
            continue;
        }
        match shuangpin::split_pinyin(group) {
            Some(v) => out.extend(v),
            None => return Vec::new(),
        }
    }
    out
}

/// 把编码拆成音节序列。返回空表示「还不能切」——调用方应再等等，而不是急着拆分。
pub fn syllables(code: &str, pinyin_mode: bool) -> Vec<String> {
    if pinyin_mode {
        full_syllables(code)
    } else {
        double_syllables(code)
    }
}

/// 把音节序列切成多段，要求**每一段都能出候选**。
///
/// 返回空 Vec 表示无法完整切分（调用方应退回逐音节）。
///
/// 用 DP 而不是贪心最长：`vhsjli` 纯贪心会切成 `vh`(张) + `sj`(三) + `li`(李)，
/// 因为 `vh` 单独就有候选。先算 `ok[i]`（从第 i 个音节起能否切完），再在可行的
/// 前提下贪心取最长段，才得到 `vhsj`(张三) + `li`(李)。
pub fn split_segments(units: &[String], has: impl Fn(&str) -> bool) -> Vec<String> {
    let n = units.len();
    if n < 2 {
        return Vec::new();
    }
    let join = |i: usize, len: usize| -> String { units[i..i + len].concat() };

    // ok[i] = 从第 i 个音节到末尾可以完整切分
    let mut ok = vec![false; n + 1];
    ok[n] = true;
    for i in (0..n).rev() {
        for len in 1..=(n - i) {
            if ok[i + len] && has(&join(i, len)) {
                ok[i] = true;
                break;
            }
        }
    }
    if !ok[0] {
        return Vec::new();
    }

    // 在 ok 的约束下贪心取最长段
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        let mut step = 0;
        for len in (1..=(n - i)).rev() {
            if ok[i + len] && has(&join(i, len)) {
                step = len;
                break;
            }
        }
        if step == 0 {
            break;
        }
        out.push(join(i, step));
        i += step;
    }
    out
}

/// 编码 → 分段。不够两个音节就返回空（单音节没有「分段」可言）。
///
/// DP 失败（某个音节在词库里完全没有）时退回逐音节：这时候拼出来的东西质量差，
/// 但**比空候选条强**——用户至少能看到自己敲的音节各自对应什么字。
pub fn plan(code: &str, pinyin_mode: bool, has: impl Fn(&str) -> bool) -> Vec<String> {
    let units = syllables(code, pinyin_mode);
    if units.len() < 2 {
        return Vec::new();
    }
    let codes = split_segments(&units, has);
    if codes.len() >= 2 {
        codes
    } else {
        units
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 双拼：两键一个音节；奇数长度说明还没打完，必须返回空而不是猜。
    #[test]
    fn double_syllables_splits_by_two_keys() {
        assert_eq!(double_syllables("vhsjli"), vec!["vh", "sj", "li"]);
        // 奇数长度：`vh` 加一个键，第二个音节还没成型
        assert!(double_syllables("vhs").is_empty(), "奇数长度不该切");
        assert!(double_syllables("vh's").is_empty(), "任一分组奇数就整体放弃");
    }

    /// 撇号是用户强制的边界，一定在那里断开；空分组（连敲两个撇号）要忽略。
    #[test]
    fn apostrophe_is_a_hard_boundary() {
        assert_eq!(double_syllables("vh'sjli"), vec!["vh", "sj", "li"]);
        assert_eq!(double_syllables("vhsj'"), vec!["vh", "sj"]);
        assert_eq!(double_syllables("'vhsj'"), vec!["vh", "sj"]);
    }

    /// 全拼：按音节表切，切不干净就整体放弃（不能返回一半）。
    #[test]
    fn full_syllables_uses_the_syllable_table() {
        assert_eq!(
            full_syllables("nihaoshijie"),
            vec!["ni", "hao", "shi", "jie"]
        );
        assert_eq!(full_syllables("xi'an"), vec!["xi", "an"]);
        assert!(full_syllables("zzz").is_empty(), "不是拼音就不该切");
    }

    /// DP 的核心承诺：**优先让整串能切完，再看单段长短**，所以是「张三|李」而不是
    /// 「张|三|李」——哪怕两者每一段都查得到。
    #[test]
    fn plan_prefers_longer_segments_when_the_rest_still_fits() {
        let units: Vec<String> = vec!["vh".into(), "sj".into(), "li".into()];
        // 假词库：单音节和「张三」都有，`vhsjli` 整串没有
        let has = |c: &str| matches!(c, "vh" | "sj" | "li" | "vhsj");
        assert_eq!(split_segments(&units, has), vec!["vhsj", "li"]);
    }

    /// 有一段的词库里完全没有 → DP 无解 → 退回逐音节（比空候选条强）。
    #[test]
    fn plan_falls_back_to_single_syllables_when_dp_fails() {
        let units: Vec<String> = vec!["vh".into(), "sj".into()];
        let has = |c: &str| c == "vh"; // `sj` 查不到任何词
        assert!(split_segments(&units, has).is_empty(), "DP 应该判定无解");
        assert_eq!(plan("vhsj", false, has), vec!["vh", "sj"], "退回逐音节");
    }

    /// 只出一个音节时不谈分段：那是「打了一半」，不是「长串」。
    #[test]
    fn plan_needs_at_least_two_syllables() {
        let has = |_: &str| true;
        assert!(plan("vh", false, has).is_empty());
        assert!(plan("vhs", false, has).is_empty(), "奇数长度也不谈分段");
    }

    /// 全拼模式下分段作用在「全拼串」上，双拼模式下作用在「双拼码」上。
    #[test]
    fn plan_follows_the_input_mode() {
        let has = |_: &str| true;
        // 全拼 `nihaoshijie` → ni|hao|shi|jie，全部候选都在 → 贪心会合成整串？
        // 不会：整串 `nihaoshijie` 也在 `has` 里（测试桩恒真），所以这里只断言
        // 「切出来的段拼回去等于原串」，不假设具体切法。
        let segs = plan("nihaoshijie", true, has);
        assert_eq!(segs.concat(), "nihaoshijie");
        assert!(segs.len() >= 2);
        // 双拼码 `vhsjli` 同样：拼回去等于原串
        let segs = plan("vhsjli", false, has);
        assert_eq!(segs.concat(), "vhsjli");
    }
}
