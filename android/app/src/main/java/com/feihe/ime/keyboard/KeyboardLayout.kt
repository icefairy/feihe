package com.feihe.ime.keyboard

/**
 * 一个键。
 *
 * `code` 是**送进引擎的东西**，不是显示的字。字母键的 code 是单个小写字母；
 * 功能键没有 code（走 [kind]）。
 *
 * 为什么显示和 code 要分开：小鹤的 `v` 键既是字母 v（ü），在双拼里又代表 zh；
 * 将来做符号页时 `，` 键的 code 可能是半角 `,`。两者相同是巧合，不能合并成一个字段。
 */
data class Key(
    val label: String,
    val kind: KeyKind,
    val code: String = "",
    /** 宽度权重（相对同一行）。空格给大一点，删除键也是。 */
    val weight: Float = 1f,
)

enum class KeyKind {
    /** 字母键：code 是一个小写字母，交给引擎 */
    CHAR,

    /**
     * **直接上屏**的字符（数字、符号）：不进组合串，也不经过引擎。
     *
     * 为什么和 [PUNCT] 分开：标点要先「把候选上屏再粘标点」，而数字页上的 `1`
     * 通常是单独一个字符（用户要打「3 个」这种），不该把前面半截拼音顶上去。
     * 两者行为不同，就不能共用一个 kind。
     */
    TEXT,

    /** 删除：短按删一个 char，长按连续删 */
    DELETE,

    /** Shift：英文态切大小写；中文态留给辅助码入口 */
    SHIFT,

    SPACE,
    ENTER,

    /** 中文标点直出（不走引擎）：`，` `。` */
    PUNCT,

    /** 中英切换 */
    MODE,

    /** 切键盘页：字母页 ↔ 数字/符号页 */
    SYMBOL,
}

/**
 * 26 键 QWERTY 布局。
 *
 * ## 为什么这里没有「小鹤双拼布局」
 *
 * 小鹤双拼**不改键位**——它改的是「按下的字母怎么解释成声母/韵母」。`b` 键上还是
 * `b`，只是引擎会把它当成声母 b；`v` 键上还是 `v`，引擎把它当成 zh。
 *
 * 所以键盘这一层必须是纯 QWERTY。任何「把韵母印到键上」的做法都会让引擎和界面
 * 各存一份映射，改一处忘一处。**键位归键盘，读音归引擎。**
 *
 * 唯一和桌面 26 键不同的是尺寸：手机每格窄得多，所以第四排（功能排）要给空格
 * 留足够宽度，否则误触率会很高。
 */
object QwertyLayout {

    private fun ch(c: Char) = Key(c.uppercaseChar().toString(), KeyKind.CHAR, c.toString())

    val rows: List<List<Key>> = listOf(
        // 第一排 10 键
        listOf('q', 'w', 'e', 'r', 't', 'y', 'u', 'i', 'o', 'p').map(::ch),

        // 第二排 9 键。真机键盘左右各留半格，手指不容易误触边缘；
        // 这里靠容器 padding 撑，不塞占位键（占位键会被读屏软件念出来）。
        listOf('a', 's', 'd', 'f', 'g', 'h', 'j', 'k', 'l').map(::ch),

        // 第三排：Shift + 7 字母 + 删除
        listOf(Key("⇧", KeyKind.SHIFT, weight = 1.5f)) +
            listOf('z', 'x', 'c', 'v', 'b', 'n', 'm').map(::ch) +
            listOf(Key("⌫", KeyKind.DELETE, weight = 1.5f)),

        // 第四排：数字页 / 中英 / 逗号 / 空格 / 句号 / 回车
        //
        // 「123」放在最左是搜狗/豆包那一排的肌肉记忆；不放的话手机上一个数字都打不出来
        // ——数字页是 26 键以外**唯一**的数字入口，字母页上没有任何地方能打 `1`。
        listOf(
            Key("123", KeyKind.SYMBOL, weight = 1f),
            Key("中", KeyKind.MODE, weight = 1f),
            Key("，", KeyKind.PUNCT, "，", weight = 0.85f),
            Key("", KeyKind.SPACE, weight = 3.3f),
            Key("。", KeyKind.PUNCT, "。", weight = 0.85f),
            Key("↵", KeyKind.ENTER, weight = 1f),
        ),
    )

    /** 所有字母键的 code。测试用：必须是 26 个互不相同的小写字母。 */
    val letterCodes: List<String>
        get() = rows.flatten().filter { it.kind == KeyKind.CHAR }.map { it.code }
}

/**
 * 数字 / 符号页。底行的 `123` 切过来，`ABC` 切回去。
 *
 * ## 为什么不是「数字候选条」而是整页
 *
 * 数字行压在字母上方会挤掉字母键的高度（每行矮 20%），而数字又不是高频输入。
 * 整页切换是手机输入法的通行做法，代价只是多按一下。
 *
 * ## 为什么带中文标点
 *
 * 打数字的场景经常连标点一起（「3.5」「第二，」）。放一页之后用户不需要为了一个
 * 逗号再切回去——**切页有成本，能一次给全就给全**。
 */
object NumericLayout {

    private fun txt(s: String) = Key(s, KeyKind.TEXT, s)

    val rows: List<List<Key>> = listOf(
        listOf("1", "2", "3", "4", "5").map(::txt),
        listOf("6", "7", "8", "9", "0").map(::txt),
        listOf("，", "。", "？", "！", "：").map(::txt),
        // 最后一行 4 个键、权重合计 5，和上面三行的 5 × 1 对齐
        listOf(
            Key("ABC", KeyKind.SYMBOL, weight = 1f),
            Key("", KeyKind.SPACE, weight = 3f),
            Key("⌫", KeyKind.DELETE, weight = 1f),
        ),
    )

    /** 所有直接上屏的字符。测试用：不许有重复，也不许混进字母。 */
    val textCodes: List<String>
        get() = rows.flatten().filter { it.kind == KeyKind.TEXT }.map { it.code }
}
