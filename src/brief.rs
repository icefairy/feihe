//! 一级简码（一简字）：26 个键各配一个「以该键开头的码里最常用的字」。
//!
//! 小鹤音形的规矩是**一码一字**：敲一个字母，那个字母代表的声母下最常用的字
//! 直接排第一候选。这是打字的人形成肌肉记忆的一部分——`d` 后面按空格，出的
//! 一定是「的」，不该因为词库换了、动态词频学歪了就变成「大」。
//!
//! ## 表从哪来
//!
//! 用的是小鹤音形官方那 26 个字（口诀「去我二人他 / 一是出哦平 / 啊三的发个 /
//! 和就可了 / 在小才这 / 不你没」）。注意几个容易记错的：
//!
//! - `f` 是**发**不是「非」——官方的 26 字口诀里 f 位就是「发」（`fei`/`fen` 那一批
//!   虽然更常用，但简码表是固定口诀，不能按词频自己挑）
//! - `v` 是**这**不是「中」——`v` 在小鹤双拼里是 `zh`，`中` 的码是 `vs` 排不进第一
//! - `x` 是**小**不是「下」——`x` 是 `x` 声母本身，`下` 的码是 `xx`
//! - `u` 是**是**——`u` 在小鹤双拼里兼 `sh`，`shi` 的码正是 `ui`
//!
//! ## 只做「提上来」，不负责「塞进去」
//!
//! `char_for` 只告诉调用方「这个键该出哪个字」，真正排序在 `dict::search` 里做，
//! 而且是**候选里本来就有它才提前**。这么设计是因为：
//!
//! - 微软双拼的键位和小鹤不同（`u` 不再是 `sh`），表里那条自然对不上，
//!   查不到就什么都不做，不会平白冒出一个不相干的字
//! - 开了辅助码筛选时（`Tab` 后敲 `b` 只要「的」系），一简字可能被筛掉了，
//!   这时也不该硬塞回来
//!
//! ## 和置顶的关系
//!
//! 优先级是「手动置顶 > 一简字 > 动态词频 > 静态词频」。一简字是**默认值**，
//! 置顶是**用户显式覆盖**——用户真想把 `d` 的第一个换成别的字，那是他的自由。

/// 一简字表：键 → 字。顺序按字母排，方便肉眼核对。
pub const TABLE: [(char, char); 26] = [
    ('a', '啊'),
    ('b', '不'),
    ('c', '才'),
    ('d', '的'),
    ('e', '二'),
    ('f', '发'),
    ('g', '个'),
    ('h', '和'),
    ('i', '出'),
    ('j', '就'),
    ('k', '可'),
    ('l', '了'),
    ('m', '没'),
    ('n', '你'),
    ('o', '哦'),
    ('p', '平'),
    ('q', '去'),
    ('r', '人'),
    ('s', '三'),
    ('t', '他'),
    ('u', '是'),
    ('v', '这'),
    ('w', '我'),
    ('x', '小'),
    ('y', '一'),
    ('z', '在'),
];

/// 这个编码对应的一简字。只有**恰好一个字母**的编码才有一简字。
///
/// 空串、`ni`、`ni'hao`、`\rq` 一律返回 `None`——一简字是「一键一字」的约定，
/// 多按一个键就进入正常的候选排序了。
pub fn char_for(code: &str) -> Option<char> {
    let mut it = code.chars();
    let c = it.next()?;
    if it.next().is_some() {
        return None;
    }
    let c = c.to_ascii_lowercase();
    TABLE.iter().find(|(k, _)| *k == c).map(|(_, v)| *v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_letter_hits_table() {
        assert_eq!(char_for("d"), Some('的'));
        assert_eq!(char_for("u"), Some('是'));
        assert_eq!(char_for("v"), Some('这'));
        assert_eq!(char_for("f"), Some('发'));
        assert_eq!(char_for("x"), Some('小'));
        // 大写也认（宿主偶尔送大写 VK）
        assert_eq!(char_for("D"), Some('的'));
    }

    #[test]
    fn longer_codes_have_no_brief() {
        // 一简字是「一键一字」，多一个键就交给正常排序
        for code in ["de", "ui", "ni", "ni'hao", "", "\r", "dd"] {
            assert_eq!(char_for(code), None, "{code} 不该有一简字");
        }
    }

    #[test]
    fn table_covers_all_26_letters() {
        let keys: String = TABLE.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, "abcdefghijklmnopqrstuvwxyz", "26 个键必须齐全且有序");
        // 不能有两个键指向同一个字（那样口诀就背不下来了）
        let mut chars: Vec<char> = TABLE.iter().map(|(_, v)| *v).collect();
        let n = chars.len();
        chars.sort_unstable();
        chars.dedup();
        assert_eq!(chars.len(), n, "一简字不能重复");
    }
}
