package com.feihe.ime.keyboard

/**
 * 一个键。
 *
 * `code` 是**送进引擎的东西**（字母键）或**直接上屏的字符**（TEXT 键）或**目标页名**
 * （PAGE 键），不是显示的字。显示的字是 [label]。
 *
 * 为什么显示和 code 要分开：小鹤的 `v` 键既是字母 v（ü），在双拼里又代表 zh；
 * `123` 键显示的是数字，code 却是 `digits`。两者相同是巧合，不能合并成一个字段。
 */
data class Key(
    val label: String,
    val kind: KeyKind,
    val code: String = "",
    /** 宽度权重（相对同一行）。空格给大一点，删除键也是。 */
    val weight: Float = 1f,
)

/** 键盘页。切页只换容器里的行，不重建 InputView。 */
enum class Page {
    LETTERS,
    DIGITS,
    SYMBOLS,
    EMOJI,
}

enum class KeyKind {
    /** 字母键：code 是一个小写字母，交给引擎 */
    CHAR,

    /**
     * **直接上屏**的字符（数字、标点、符号）：不进组合串，也不经过引擎。
     *
     * 为什么和 [PUNCT] 分开：中文标点要先「把候选上屏再粘标点」，而数字页上的 `1`
     * 通常是单独一个字符（用户要打「3 个」这种），不该把前面半截拼音顶上去。
     * 两者行为不同，就不能共用一个 kind。
     */
    TEXT,

    /** 删除：短按删一个 char，长按连续删 */
    DELETE,

    /** 上档（一次性）：按一下点亮，下一个字母大写，打完自动落下 */
    SHIFT,

    SPACE,
    ENTER,

    /** 中文标点直出（不走引擎）：`，` `。` */
    PUNCT,

    /** 中英切换 */
    MODE,

    /**
     * 换页。code 是 [Page] 的小写名（`letters` / `digits` / `symbols`）。
     *
     * 不用「布尔值切两页」是因为有了第三页：符号单独一页之后，`123`/`ABC`/`符`
     * 三个键要去三个地方，布尔值装不下，硬塞只会变成一堆 if。
     */
    PAGE,

    /**
     * 数字选词：code 是 `"1"`..`"9"`，点它选第 N 个候选。
     *
     * 和 [TEXT] 的区别同 [PUNCT]：TEXT 是"直接上屏这个字符"，SELECT 是"拿这个数字
     * 当候选下标"。两者行为完全不同，共用一个 kind 就会在数字页上打 `1` 时误选候选。
     * 这一行只在有候选时出现，详见 [NumberSelectLayout] 和 `docs/后续待办.md` A2。
     */
    SELECT,
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
        // 「123」放在最左是搜狗/豆包那一排的肌肉记忆；字母页上没有任何地方能打
        // 数字，这个键是唯一入口。
        listOf(
            Key("123", KeyKind.PAGE, "digits", weight = 1f),
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
 * 数字页：**3 列 × 3 行的九宫格（1–9）+ 0/逗号/句号 + 一排功能键**。
 *
 * ## 为什么改成三列
 *
 * 上一版是每行 5 个键（`1 2 3 4 5` / `6 7 8 9 0`），每个键只有屏宽的 1/5，
 * 手指按上去经常落偏；而手机上最常见的数字键盘是电话拨号盘那种三列布局，
 * 键宽接近 1/3，单手按也不容易错。这是这次改动的全部理由——没有新功能。
 *
 * ## 为什么多了一排却不让键盘变高
 *
 * 字母页 4 排、数字页 5 排。如果行高都是 46dp，切页时整块键盘会长出 50dp，
 * 输入框和候选条跟着往上跳一下，很烦。所以数字/符号页用 36dp 的紧凑行高：
 *
 * - 字母页：4 × 46 + 3 × 5(缝) + 12(上下 padding) = **211dp**
 * - 紧凑页：5 × 36 + 4 × 5(缝) + 12(上下 padding) = **212dp**
 *
 * 差 1dp，肉眼看不出来，而每个键还比原来宽 2/3。行高由 [KeyboardPages.rowHeightDp] 给。
 *
 * ## 为什么要有独立符号页
 *
 * 三列 × 五行 = 15 格，装不下「10 个数字 + 5 个中文标点 + 5 个功能键」。
 * 硬塞就得把某一排做成 6 个小键，等于把刚争取到的键宽又还回去。
 * 标点单独一页（[SymbolLayout]），数字页只留 `，` `。` 两个最常用的。
 */
/**
 * 紧凑页（数字 / 符号 / 表情）共用的底部功能排。
 *
 * ## 为什么单独抽出来
 *
 * 这一排原来写了两份（数字页一份、符号页一份），加表情页时变成三份。三份一样的东西
 * 抄三遍，改空格键的宽度就得改三处，漏一处就是「切到表情页空格忽然变窄」。
 *
 * ## 为什么权重是 0.8 / 2 / 0.8 而不是全 1
 *
 * 行内按键按 `weight / 行权重合计` 分宽度，各行合计相同、键宽才对得齐。正文行
 * （3 列 × 2）合计 6，所以这一排也必须合计 6：五个功能键各 0.8（4）+ 空格 2 = 6。
 *
 * 空格给 2（占三分之一宽）是刻意的：它是这一排里唯一会被「盲按」的键，
 * 太窄会频繁点到 ⌫。
 */
private fun compactFunctionRow(nav: List<Key>): List<Key> {
    require(nav.size == 3) { "紧凑页的功能排要放 3 个换页键，实际 ${nav.size} 个" }
    return nav + listOf(
        Key("", KeyKind.SPACE, weight = 2f),
        Key("↵", KeyKind.ENTER, weight = 0.8f),
        Key("⌫", KeyKind.DELETE, weight = 0.8f),
    )
}

/** 三个换页键的公共定义（权重统一 0.8，见 [compactFunctionRow]） */
private val NAV_LETTERS = Key("ABC", KeyKind.PAGE, "letters", weight = 0.8f)
private val NAV_DIGITS = Key("123", KeyKind.PAGE, "digits", weight = 0.8f)
private val NAV_SYMBOLS = Key("符", KeyKind.PAGE, "symbols", weight = 0.8f)
private val NAV_EMOJI = Key("😊", KeyKind.PAGE, "emoji", weight = 0.8f)

object NumericLayout {

    private fun txt(s: String) = Key(s, KeyKind.TEXT, s, weight = 2f)

    val rows: List<List<Key>> = listOf(
        listOf("1", "2", "3").map(::txt),
        listOf("4", "5", "6").map(::txt),
        listOf("7", "8", "9").map(::txt),
        listOf("，", "0", "。").map(::txt),
        // 功能排：和数字行一样按「总权重 6」分，否则这一排的键宽会和上面跳一格。
        // `符` 去符号页，`ABC` 回字母页，`123` 在符号页上用来回数字页。
        compactFunctionRow(listOf(NAV_LETTERS, NAV_SYMBOLS, NAV_EMOJI)),
    )

    /** 所有直接上屏的字符。测试用：不许有重复，也不许混进字母。 */
    val textCodes: List<String>
        get() = rows.flatten().filter { it.kind == KeyKind.TEXT }.map { it.code }
}

/**
 * 中文标点 / 常用符号页。形状和数字页完全一样（5 行、每行总权重 6），
 * 这样两页之间切换时**每一个键的位置都不动**，肌肉记忆能直接复用。
 */
object SymbolLayout {

    private fun txt(s: String) = Key(s, KeyKind.TEXT, s, weight = 2f)

    val rows: List<List<Key>> = listOf(
        listOf("，", "。", "？").map(::txt),
        listOf("！", "：", "、").map(::txt),
        listOf("；", "（", "）").map(::txt),
        listOf("…", "—", "·").map(::txt),
        compactFunctionRow(listOf(NAV_LETTERS, NAV_DIGITS, NAV_EMOJI)),
    )

    val textCodes: List<String>
        get() = rows.flatten().filter { it.kind == KeyKind.TEXT }.map { it.code }
}

/**
 * 表情页。
 *
 * ## 为什么复用 [KeyKind.TEXT] 而不是新加一个 EMOJI kind
 *
 * emoji 在本项目里**就是一个普通字符**：不经过引擎、不进组合串、直接 commitText。
 * 这正是 TEXT 的语义。新加一个 kind 只会让 onKey 里多一个和 TEXT 完全一样的分支。
 *
 * ## 为什么只有 28 个、不做表情分类/翻页
 *
 * 手机自带的 emoji 键盘（长按地球键切出来的那个）有全套 + 搜索 + 最近使用，做得
 * 比这好得多，且不占我们的包体。这一页的定位是**常用表情的快捷入口**——
 * 用户要打 😂 不该先切出去。所以刻意控制在一屏内，不做滚动、不做分组。
 * 想加表情就往数据里加，加满一屏为止。
 *
 * ## 为什么底部功能排和数字/符号页一模一样
 *
 * 三页共用 5 行 × 36dp（见 [NumericLayout] 里的高度计算），换页时键盘高度不变；
 * 底部 ABC / 123 / 符 / 空格 / ↵ / ⌫ 位置固定，肌肉记忆直接复用。
 *
 * ## 为什么是 7 列（数字/符号页只有 3 列）
 *
 * 列数跟着内容走：数字页 3 列是为了把键撑宽、防误触，表情却是**越多越好选**，
 * 一屏塞得下就不该翻页。7 列 × 4 行 = 28 个，键宽仍够手指点。行列不同不影响
 * 高度——高度只由「行数 × 行高」决定，所以切页时键盘不会跳。
 *
 * 不变的是**每行的权重合计**（都是 6），这才是「各行键宽看着一样」的真正约束。
 */
object EmojiLayout {

    /**
     * 每个表情键的权重。
     *
     * 一行 7 个键，而各行的**权重合计必须和别的页一样是 6**——否则同一个 weight
     * 在表情页和数字页的物理宽度不同，切页时手指会落偏。6 / 7 正好让七个键铺满一行。
     */
    private const val KEY_WEIGHT = 6f / 7f

    private fun e(s: String) = Key(s, KeyKind.TEXT, s, weight = KEY_WEIGHT)

    val rows: List<List<Key>> = listOf(
        listOf("😀", "😄", "😂", "🤣", "😊", "😍", "🤔").map(::e),
        listOf("😅", "😭", "😡", "😴", "👍", "👌", "👏").map(::e),
        listOf("🙏", "💪", "🤝", "❤️", "💔", "🎉", "🔥").map(::e),
        listOf("✨", "⭐", "✅", "❌", "❓", "❗", "💯").map(::e),
        compactFunctionRow(listOf(NAV_LETTERS, NAV_DIGITS, NAV_SYMBOLS)),
    )

    val textCodes: List<String>
        get() = rows.flatten().filter { it.kind == KeyKind.TEXT }.map { it.code }
}

/**
 * 数字选词行：**只在有候选时出现的一排 `1`–`9`**。
 *
 * ## 为什么不把它做成键盘的常驻第五排
 *
 * A2 那条待办犹豫的是"加一行会挤掉字母高度"。这里的解法是**不进键盘**：
 * 它是候选条和键盘之间一个独立的、高度为 0 的容器，[rows] 只在候选非空时才被
 * 加进去，没候选时整行 `GONE`。所以字母键的高度一 dp 都没少，代价只是组合态期间
 * 多一行 30dp——而那时用户本来就在看候选条。
 *
 * 设置里可以整个关掉（"可隐藏"），关掉后连组合态也不出现。
 *
 * 一行最多 9 个键，按 `weight=1` 均分，和字母页 `weightSum` 的规则一致。
 */
object NumberSelectLayout {
    /** 由候选数决定这一行的键。空表表示"这一行不显示"。 */
    fun rows(candidateCount: Int): List<List<Key>> {
        val digits = NumberSelect.digits(candidateCount)
        if (digits.isEmpty()) return emptyList()
        return listOf(digits.map { Key(it.toString(), KeyKind.SELECT, it.toString()) })
    }

    /** 比键盘行矮：它只是"候选的快捷入口"，不该和字母键抢视觉重量。 */
    const val ROW_HEIGHT_DP = 30f
}

/** 给壳用的一张小表：页 → 行数据 + 行高。避免 FeiheIme 里散落三个 when。 */
object KeyboardPages {
    fun rows(page: Page): List<List<Key>> = when (page) {
        Page.LETTERS -> QwertyLayout.rows
        Page.DIGITS -> NumericLayout.rows
        Page.SYMBOLS -> SymbolLayout.rows
        Page.EMOJI -> EmojiLayout.rows
    }

    /** 见 [NumericLayout] 里的高度计算：紧凑页 36dp，总和与字母页 46dp×4 相当。 */
    fun rowHeightDp(page: Page): Float = if (page == Page.LETTERS) 46f else 36f

    fun label(page: Page): String = when (page) {
        Page.LETTERS -> "26 键"
        Page.DIGITS -> "数字"
        Page.SYMBOLS -> "符号"
        Page.EMOJI -> "表情"
    }
}
