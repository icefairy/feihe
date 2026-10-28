//! 易错词的读音提示 + 错音容错。
//!
//! 词库存的是**不带调**的全拼（`chong qing`），所以有两件事它做不到：
//!
//! 1. 用户看不出「重庆」读 chóng 还是 zhòng；
//! 2. 多音字词只挂一个读音，按另一个读音打永远打不出来——打 `zhongqing`
//!    出的是「钟情」，「重庆」不出现。用户并不知道自己读错了，只会以为没这个词。
//!
//! 两个问题都由 `src/hint.txt` 解决，那是一张**自动生成的小表**（见
//! `tools/gen_hint.py`）：每行「词 / 带调读音 / 按字级默认读音拼出来的全拼」。
//! 只有「词库读音 ≠ 该字默认读音」的词才进来，全库 4.5 万条里只有 753 条。
//!
//! ## 为什么不在 `dict::search` 里加
//!
//! 容错候选必须**追加在候选末尾**，绝不能参与 `search` 的排序与提权——尤其不能
//! 影响 `lookup(code, 1)`。那个调用在分词、自造词、二简词里被当成「这个码有词吗」
//! 的判断用，往里掺容错会让 `zhongqing` 被当成「有词」，分词结果跟着变。
//! 所以这里只提供查询，由 `textservice::refresh` 在候选定稿之后追加。
//!
//! ## 前缀匹配
//!
//! 变体码是**整词**的码（`zhongqing`），查询按前缀匹配，所以打到 `zhongq` 也
//! 能出「重庆」。三套索引（小鹤 / 微软 / 全拼）在首次使用时各建一份并排序，
//! 之后每次按键只是一次二分。

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::shuangpin::{self, Scheme};

/// 表内容。`include_str!` 而不是构建期编成二进制：26 KB，直接躺在 `.rdata` 里，
/// 由 PE 加载器映射，多进程共享同一份物理页；为这点数据再造一种二进制格式不划算。
const RAW: &str = include_str!("hint.txt");

struct Table {
    /// 词 → 带调读音
    toned: HashMap<&'static str, &'static str>,
    /// 三套「错音码 → 词」索引，各自按码排序（查询靠二分）
    wrong_xh: Vec<(String, &'static str)>,
    wrong_ms: Vec<(String, &'static str)>,
    wrong_flat: Vec<(String, &'static str)>,
}

fn table() -> &'static Table {
    static T: OnceLock<Table> = OnceLock::new();
    T.get_or_init(|| {
        let mut toned = HashMap::new();
        let mut wrong_xh = Vec::new();
        let mut wrong_ms = Vec::new();
        let mut wrong_flat = Vec::new();

        for line in RAW.lines() {
            let line = line.trim_end_matches('\r');
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut it = line.split('\t');
            let (Some(word), Some(py)) = (it.next(), it.next()) else {
                continue;
            };
            if word.is_empty() || py.is_empty() {
                continue;
            }
            toned.insert(word, py);

            // 第三列：用户可能按错的读音。没有就说明「按错音打也照样能出」，
            // 不需要容错条目。
            let Some(wrong) = it.next() else { continue };
            let wrong = wrong.trim();
            if wrong.is_empty() {
                continue;
            }
            if let Some(code) = shuangpin::encode_pinyin(Scheme::Xiaohe, wrong) {
                wrong_xh.push((code, word));
            }
            if let Some(code) = shuangpin::encode_pinyin(Scheme::Ms, wrong) {
                wrong_ms.push((code, word));
            }
            let flat: String = wrong.split_whitespace().collect();
            if !flat.is_empty() {
                wrong_flat.push((flat, word));
            }
        }

        wrong_xh.sort();
        wrong_ms.sort();
        wrong_flat.sort();
        Table {
            toned,
            wrong_xh,
            wrong_ms,
            wrong_flat,
        }
    })
}

/// 某个词的带调读音。不在表里（= 不是易错词）返回 `None`。
pub fn toned(word: &str) -> Option<&'static str> {
    table().toned.get(word).copied()
}

/// 错音容错：`code` 前缀能命中的词，最多 `limit` 个。
///
/// `flat` = 全拼模式（码是去掉空格的全拼）。返回顺序按码，稳定。
///
/// 码太短（1~2 键）时直接返回空：变体码最短的是两字词（双拼 4 键），敲一个键就要
/// 命中一大堆，「重庆」会跟着 `z` `zh` 这种前缀冒出来，纯噪音。三个键起才开始给。
pub fn wrong_reading(code: &str, flat: bool, limit: usize) -> Vec<&'static str> {
    if code.chars().count() < 3 || limit == 0 {
        return Vec::new();
    }
    let t = table();
    let idx = if flat {
        &t.wrong_flat
    } else {
        match crate::dict::scheme() {
            Scheme::Xiaohe => &t.wrong_xh,
            Scheme::Ms => &t.wrong_ms,
        }
    };
    // 第一个 >= code 的位置；它前面的都不可能以 code 开头
    let start = idx.partition_point(|(c, _)| c.as_str() < code);
    idx[start..]
        .iter()
        .take_while(|(c, _)| c.starts_with(code))
        .map(|(_, w)| *w)
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 表本身要自洽：词不重复、读音非空、音节数等于字数。
    /// 这条挡住的是「生成脚本改坏了」——它不会在任何运行时路径上报错，
    /// 只会表现为某个词突然不提示读音或者提示一串错位的东西。
    #[test]
    fn table_is_consistent() {
        let t = table();
        assert!(t.toned.len() > 500, "表太小：{} 条", t.toned.len());
        for (w, py) in &t.toned {
            assert!(!w.is_empty() && !py.is_empty());
            assert_eq!(
                w.chars().count(),
                py.split_whitespace().count(),
                "「{w}」的读音音节数和字数对不上：{py}"
            );
        }
    }

    /// 带调读音必须带调。没有调号说明生成时用了 NORMAL 样式，
    /// 那提示出来的东西和用户看到的拼音完全一样，白占地方。
    #[test]
    fn readings_are_toned() {
        let marks = "āáǎàēéěèīíǐìōóǒòūúǔùǖǘǚǜüńňǹ";
        let t = table();
        let toned = t
            .toned
            .values()
            .filter(|py| py.chars().any(|c| marks.contains(c)))
            .count();
        // 极少数轻声词（「便宜」的 yi）整词没调号，所以不能要求 100%
        assert!(
            toned * 10 >= t.toned.len() * 9,
            "带调比例太低：{toned}/{}",
            t.toned.len()
        );
    }

    /// 关键样例：这几个是读音最容易错、也最容易被改成错的。
    #[test]
    fn known_words_have_right_reading() {
        for (w, want) in [
            ("重庆", "chóng qìng"),
            ("弹窗", "tán chuāng"),
            ("水雷屯", "shuǐ léi zhūn"),
            ("川芎", "chuān xiōng"),
            ("六安", "lù ān"),
        ] {
            assert_eq!(toned(w), Some(want), "「{w}」的读音不对");
        }
        // 读音等于字级默认的词不该进表（进了就是噪音）
        assert_eq!(toned("中国人民"), None);
    }

    /// 错音容错：按默认读音打也要能查到「重庆」。
    #[test]
    fn wrong_reading_finds_word() {
        let xh = shuangpin::encode_pinyin(Scheme::Xiaohe, "zhong qing").expect("编码失败");
        assert!(
            wrong_reading(&xh, false, 4).contains(&"重庆"),
            "小鹤 {xh} 查不到重庆"
        );
        let ms = shuangpin::encode_pinyin(Scheme::Ms, "zhong qing").expect("编码失败");
        // 不走 `wrong_reading`：它按**当前**方案选索引，而方案是全局状态，
        // 测试里改来改去会和别的用例抢。这里直接验证微软那份索引建对了。
        assert!(
            table().wrong_ms.iter().any(|(c, w)| *c == ms && *w == "重庆"),
            "微软索引里没有 {ms} → 重庆"
        );
        assert!(
            wrong_reading("zhongqing", true, 4).contains(&"重庆"),
            "全拼查不到重庆"
        );
        // 前缀（还没打完）也要命中
        assert!(
            wrong_reading("zhongq", true, 4).contains(&"重庆"),
            "全拼前缀查不到重庆"
        );
        // 不相干的码不该乱给
        assert!(wrong_reading("nihao", true, 4).is_empty());
        assert!(wrong_reading("", false, 4).is_empty());
        // 太短的前缀不查：否则敲第一个键「重庆」就跟出来了
        assert!(wrong_reading("zh", true, 4).is_empty());
    }

    /// 同一个错音码的变体数不能超过调用方要的数量。
    ///
    /// `textservice::append_wrong_reading` 传的是 `limit = 2`，超出的部分会被
    /// **静默丢掉**——和「玑」被候选上限吃掉是同一类问题：字没丢，但永远出不来。
    /// 哪天往 `hint.txt` 里加词把某个码凑到 3 条，这条会红，提醒把调用处的
    /// limit 一起放开（或者在生成脚本里就限制成 2 条）。
    #[test]
    fn wrong_codes_fit_the_callers_limit() {
        let t = table();
        let mut per: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for (code, _) in &t.wrong_flat {
            *per.entry(code.as_str()).or_default() += 1;
        }
        let worst = per
            .iter()
            .max_by_key(|(_, n)| **n)
            .map(|(c, n)| format!("{c} ×{n}"))
            .unwrap_or_default();
        assert!(
            per.values().all(|&n| n <= 2),
            "有错音码超过 2 条（最坏 {worst}），append_wrong_reading 的 limit=2 会丢掉后面的"
        );
    }
}
