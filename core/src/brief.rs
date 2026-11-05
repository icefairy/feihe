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

/// 一简次选表：键 → 次选字/词。
///
/// 官方表里每个键还有**第二个**候选（`d` 的第二位是「打」、`c` 是「词库」），
/// 小鹤用分号键 `;` 直接取（`d;` → 打）。飞鹤同样支持：`d;` 上屏次选，
/// 不按 `;` 的话它就老实待在第二位。
///
/// 官方说明「放用户分类词库中，不需要可清空」——这里同理，整张表留空就等于关掉。
pub const ALT: [(char, &str); 26] = [
    ('a', "安装"),
    ('b', "比较"),
    ('c', "词库"),
    ('d', "打"),
    ('e', "嗯嗯"),
    ('f', "复杂"),
    ('g', "管"),
    ('h', "忽略"),
    ('i', "重新"),
    ('j', "见"),
    ('k', "可以"),
    ('l', "理解"),
    ('m', "没有"),
    ('n', "内容"),
    ('o', "哦哦"),
    ('p', "便宜"),
    ('q', "期"),
    ('r', "然后"),
    ('s', "所有"),
    ('t', "提供"),
    ('u', "时"),
    ('v', "之"),
    ('w', "位"),
    ('x', "显示"),
    ('y', "有时"),
    ('z', "自动"),
];

/// 这个编码对应的**一简次选**。同样只认恰好一个字母。
pub fn alt_for(code: &str) -> Option<&'static str> {
    let mut it = code.chars();
    let c = it.next()?;
    if it.next().is_some() {
        return None;
    }
    let c = c.to_ascii_lowercase();
    ALT.iter().find(|(k, _)| *k == c).map(|(_, v)| *v)
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
    fn alt_table_covers_all_and_is_unique() {
        let keys: String = ALT.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, "abcdefghijklmnopqrstuvwxyz", "26 个键必须齐全且有序");
        let mut words: Vec<&str> = ALT.iter().map(|(_, v)| *v).collect();
        let n = words.len();
        words.sort_unstable();
        words.dedup();
        assert_eq!(words.len(), n, "次选词不能重复");
        // 次选和首选也不能撞：`d` 的第二位不该又是「的」
        for (_, w) in ALT {
            assert!(
                !TABLE.iter().any(|(_, c)| c.encode_utf8(&mut [0u8; 4]) == w),
                "{w} 同时出现在首选和次选表里"
            );
        }
    }

    #[test]
    fn alt_hits_only_single_letter() {
        assert_eq!(alt_for("d"), Some("打"));
        assert_eq!(alt_for("j"), Some("见"));
        assert_eq!(alt_for("c"), Some("词库"));
        assert_eq!(alt_for("D"), Some("打"));
        for code in ["", "da", "ni'hao", "dd"] {
            assert_eq!(alt_for(code), None, "{code} 不该有次选");
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
