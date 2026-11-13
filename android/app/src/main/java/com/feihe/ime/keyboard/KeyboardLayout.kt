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
    /**
     * 上滑输入的字符。不为 null 时键帽右上角会画一行小字（淡色），在这个键上
     * 向上滑就把它上屏。
     *
     * 只给 26 个字母键配了：功能键（⇧/⌫/空格/回车）上滑没有意义，而且它们还要
     * 长按（连删、锁定），两套手势叠在一起必然互相抢。
     */
    val swipe: String? = null,
)

/** 键盘页。切页只换容器里的行，不重建 InputView。 */
enum class Page {
    LETTERS,
    DIGITS,
    SYMBOLS,
    EMOJI,

    /** 光标与编辑控制（方向键 / 剪切复制粘贴 / 全选）。 */
    NAV,
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

    /** 上档（一次性）：按一下点亮，下一个字母大写，打完自动落下；双击锁定 */
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

    /**
     * 光标 / 编辑控制键。code 是 [NavAction] 的枚举名。
     *
     * ## 为什么必须是独立的 kind
     *
     * 上下左右和复制粘贴**都不能用 commitText 实现**：光标移动要发 KEYCODE_DPAD_*，
     * 复制粘贴要调 InputConnection.performContextMenuAction。用 TEXT 会变成往文本框
     * 里插入一串字符——这正是「点了左键结果打了个箭头符号」的那种 bug。
     *
     * 而且光靠 INJECT_EVENTS 未必够：某些宿主（尤其是 Electron/WebView 里的输入框）
     * 对 KEYCODE_DPAD_LEFT 没反应，只认 setSelection。所以实现里两条路都走。
     */
    NAV,
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

    /**
     * 26 个字母键的**上滑字符**（键帽右上角那行小字）。
     *
     * 排布照手机中文输入法的惯例：第一排 1–0，第二排符号，第三排中文最常用的标点。
     *
     * ## 为什么第三排是 9 个而不是 7 个
     *
     * 字母页底部原来还有 `，` `、` `。` 三个独立标点键，挤在空格两边。这一版把它们
     * **全部收进上滑**（`c` `m` `v`），空格两侧只留功能键——拇指盲按空格时不再
     * 蹭到标点，误触率直降。代价是标点得滑一下才出来，所以第三排的配额必须放宽：
     *
     * - `z x c v` = `！？，。`：中文断句四件套，滑的距离最短、位置最好记
     * - `b n m` = `：；、`：次常用的三个
     * - **`/` 单独占 `l` 的上滑**：以前整个键盘都打不出斜杠（只能去符号页），
     *   而它其实是输入网址、路径、日期时最刚需的符号之一
     *
     * **只在这一张表里改**：键帽小字、上滑动作、单测三处都读它。
     */
    val SWIPE: Map<Char, String> = mapOf(
        // 第一排：数字 1-0（和豆包一致）
        'q' to "1", 'w' to "2", 'e' to "3", 'r' to "4", 't' to "5",
        'y' to "6", 'u' to "7", 'i' to "8", 'o' to "9", 'p' to "0",
        // 第二排：**照豆包**的 `- / : ; ( ) ~ “ ”`。
        //
        // 原来是 `@ # ¥ % & * - ( /`——一排里塞了 ¥ % & * 四个低频符号，
        // 而 `: ; ( ) ~` 和中文双引号反而一个都没有，打网址/代码/引用时要
        // 反复进符号页。豆包这排是「程序员常用 + 中文引号」的组合，覆盖面
        // 明显更好，且用户明确说这套用惯了。
        'a' to "-", 's' to "/", 'd' to ":", 'f' to ";", 'g' to "(",
        'h' to ")", 'j' to "~", 'k' to "“", 'l' to "”",
        // 第三排：中文态保留**中文标点**（这是中文打字的刚需，豆包在这排放
        // 的是 `@ . # 、` 这类半角符号，我们已在符号页覆盖）。
        // 英文态见 SWIPE_EN——那里换成豆包的半角组合。
        // **`c` / `v` 上的 `，` `。` 已让位给底排独立标点键**（用户要求），
        // 换成中文里同样高频、但键盘上没地方直接打的两个：
        //   `c` → `《`  书名号左半。写书名/文章名是刚需，符号页里要翻两页
        //   `v` → `》`  和 `c` 成对，同一排相邻，肌肉记忆好记
        // 选它们不选 `「」`：中文正文里《》出现频率更高（书籍、影视、文件名）。
        'z' to "！", 'x' to "？", 'c' to "《", 'v' to "》",
        'b' to "：", 'n' to "；", 'm' to "、",
    )

    /**
     * 英文模式的上滑标点。
     *
     * ## 三层不重复原则（推倒重排过两次）
     *
     * 英文态能打标点的地方有三处：**字母键上滑**、**底排标点键**、**符号页**。
     * 以前三处各排各的，结果 `,` 和 `.` 各出现 **3 次**，`! " ' ( ) - / : ; ?`
     * 各出现 2 次——用户切到英文要打某个标点，同一条命在三个地方都能摸到，
     * 真正缺的（`@ # $ & * + = [ ] { } \ | ` ~`）却要去符号页翻。
     *
     * 而且中文表里 K/L 是弯引号 `“ ”`（中文排版用），英文必须换成直引号 `' "`；
     * 这和第三排的全角/半角是同一类问题——弯引号进代码、JSON、命令行都是非法字符。
     *
     * ## 现在的分工
     *
     * - **底排标点键**：`,` 上滑 `.`。这两个是英文写作频率最高的，给独立键。
     * - **第二排 `a s d f g h j k l`**：`- / : ; ( ) ~ ' "`（豆包实测同款）。
     * - **第三排 `z x c v b n m`**：补上"键盘上根本打不出来"的程序员符号。
     *
     * 第三排重排的依据：这些符号以前**只能进符号页**打，而写代码/命令行天天用；
     * 反过来 `,` `.` 已经占了底排独立键，`? !` 在符号页第一屏就能点到，不该重复占位。
     */
    val SWIPE_EN: Map<Char, String> = SWIPE + mapOf(
        // 第二排：只换两个引号，其余（- / : ; ( ) ~）本来就是半角，直接沿用。
        // 注意这里 `-` `/` `:` `;` `(` `)` 与符号页有重叠，但它们是**不同键位上的
        // 同族符号**，属于刻意保留的高频冗余（手在字母区时不必跳页），
        // 与"同一符号在同一个手势层重复"是两回事。
        'k' to "'", 'l' to "\"",
        // 第三排：字母区原本打不到、且英文写作/写代码高频的半角符号。
        //
        //   `?` `!` `~`     英文标点和家目录，**必保**（原来就在，不该白丢）
        //   `@` `#` `$`     邮箱/井号/CSS颜色/变量
        //   `*` `…`         通配/省略号（`…` 是英文排版里的三连点，不能丢）
        'z' to "?", 'x' to "!", 'c' to "@",
        'v' to "#", 'b' to "$", 'n' to "*",
        'm' to "…", 'j' to "~",
        // 让位说明：`&` `%` `_` 从字母区退到符号页。
        // 26 个字母键是固定槽位，符号永远放不全；这里按**使用频率**取舍：
        // `? ! ~` 是句子级刚需（问句、感叹、家目录），而 `& % _` 有替代路径
        // （符号页第一屏、或 `,`/`.` 键的上滑）。
        // 紧邻 `n`（`*`）和 `m`（`=`）的 `&` `_` 反而更该在实际打字里让位。
    )

    /** 按中英模式取上滑表。英文用半角，中文用全角。 */
    fun swipeMap(chinese: Boolean): Map<Char, String> =
        if (chinese) SWIPE else SWIPE_EN

        private fun ch(c: Char, chinese: Boolean = true) =
        Key(
            c.uppercaseChar().toString(),
            KeyKind.CHAR,
            c.toString(),
            swipe = swipeMap(chinese)[c],
        )

    private val CHINESE_ROWS: List<List<Key>> = listOf(
        // 第一排 10 键
        listOf('q', 'w', 'e', 'r', 't', 'y', 'u', 'i', 'o', 'p').map{ ch(it, true) },

        // 第二排 9 键。真机键盘左右各留半格，手指不容易误触边缘；
        // 这里靠容器 padding 撑，不塞占位键（占位键会被读屏软件念出来）。
        listOf('a', 's', 'd', 'f', 'g', 'h', 'j', 'k', 'l').map{ ch(it, true) },

        // 第三排：Shift + 7 字母 + 删除
        //
        // 关于底排标点键：`，` `、` `。` 原来在这里占三个位置（夹在空格两边），已按
        // 用户要求**全部去掉**，改由 `c`/`m`/`v` 上滑输入。空格两侧现在只有功能键，
        // 拇指盲按时不会再蹭到标点。
        listOf(Key("⇧", KeyKind.SHIFT, weight = 1.5f)) +
            listOf('z', 'x', 'c', 'v', 'b', 'n', 'm').map{ ch(it, true) } +
            // **退格回到 1.5，和 ⇧ 对称**。
            //
            // 曾经想把它压到 1、把回车抬到 1.5，但那样第三排合计变成 9.5，
            // 和第一/四排的 10 差 0.5，Z 排每个字母键会比 Q 排窄 1.5%——
            // 换页时键宽会跳，看着像没对齐。
            //
            // 所以两个键各占各的：退格管第三排右端（和左侧 ⇧ 对称，
            // 视觉平衡），回车管第四排右端（1.5，比原来 1 宽一半）。
            // 两排都是 10，键宽全键盘一致。
            listOf(Key("⌫", KeyKind.DELETE, weight = 1.5f)),

        // 第四排：数字页 / 中英 / 表情 / 控制 / 空格 / 回车
        //
        // 原来这里是「123 中 😊 ，、␣ 。↵」，去掉三个标点后空出两个位置，正好装下
        // 「导航」页入口。
        //
        // 参照豆包输入法的底排配比（用户实测那套布局用着习惯）：
        //
        //   豆包：123 | , | 空格 | 中/英 | 换行
        //   实测像素比（以字母键为 1）：123≈2.2  空格≈4.75  中英≈1.3  换行≈1.76
        //
        // 我们多两个键（😊 表情、⌘ 导航），所以把豆包的比例摊薄后取：
        //
        //   123(1.6) | 😊(0.7) | 空格(4.2) | 中(1.0) | ⌘(0.7) | ↵(1.8)  = 10.0
        //
        // 两个要点来自豆包：
        // 1. **回车 1.8**，明显宽于其它功能键。原来给 1.0（和表情键一样宽），
        //    按偏就白打一行——这是用户抱怨「回车太窄」的根。
        // 2. **「中」挪到空格右边**。豆包/ iOS 都是这么放的，右手拇指够得着；
        //    原来在左边，切中英要横跨整个空格。
        //
        // 空格不追求左右严格对称：豆包自己也是左 2.2 / 右 3.0（空格中心比
        // 屏幕中线偏左约 5%），实测没人觉得歪。这里左 2.3 / 右 3.5，
        // 偏移量同量级。硬要对称就得把回车压窄，那是退步。
        // `123` 从 1.6 压到 1.0：用户反馈底排数字键太占地方。腾出的 0.6
        // 正好给新增的标点键——它在 `123` 和 `😊` 之间，是右手拇指的自然落点。
        functionRow(true),
    )

    /**
     * 底排的标点键：中文全角 `，` / 上滑 `。`，英文半角 `,` / 上滑 `.`。
     *
     * **必须按语言分开**：英文输入打全角逗号是错的（邮箱、网址、命令行、
     * JSON 里全角标点一律是非法字符）。以前整张底排共用一份，切英文后
     * 键帽还是全角，用户打 `1,000` 得到的是 `1，000`。
     */
    private fun punctKey(chinese: Boolean): Key =
        if (chinese) Key("，", KeyKind.PUNCT, "，", weight = 1f, swipe = "。")
        else Key(",", KeyKind.PUNCT, ",", weight = 1f, swipe = ".")

    /** 底排（中英共用结构，只有标点键的语言不同）。 */
    private fun functionRow(chinese: Boolean): List<Key> = listOf(
        Key("123", KeyKind.PAGE, "digits", weight = 1f),
        punctKey(chinese),
        Key("😊", KeyKind.PAGE, "emoji", weight = 0.7f),
        Key("", KeyKind.SPACE, weight = 3.9f),
        Key("中", KeyKind.MODE, weight = 1f),
        Key("⌘", KeyKind.PAGE, "nav", weight = 0.7f),
        Key("↵", KeyKind.ENTER, weight = 1.8f),
    )

    /** 英文模式：字母键的上滑改半角标点，其余结构完全一致。 */
    private val ENGLISH_ROWS: List<List<Key>> = listOf(
        listOf('q', 'w', 'e', 'r', 't', 'y', 'u', 'i', 'o', 'p').map{ ch(it, false) },
        listOf('a', 's', 'd', 'f', 'g', 'h', 'j', 'k', 'l').map{ ch(it, false) },
        listOf(Key("⇧", KeyKind.SHIFT, weight = 1.5f)) +
            listOf('z', 'x', 'c', 'v', 'b', 'n', 'm').map{ ch(it, false) } +
            listOf(Key("⌫", KeyKind.DELETE, weight = 1.5f)),
        functionRow(false),
    )

    val rows: List<List<Key>> get() = CHINESE_ROWS

    /** 按中英模式取行数据。中英切换要重建键盘才能换掉键帽小字和上滑内容。 */
    fun rowsOf(chinese: Boolean): List<List<Key>> =
        if (chinese) CHINESE_ROWS else ENGLISH_ROWS

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
private val NAV_NAV = Key("⌘", KeyKind.PAGE, "nav", weight = 0.8f)

object NumericLayout {

    /**
     * 数字键的上滑字符：**运算符号 + 最常用的两个标点**。
     *
     * ## 为什么是这一组
     *
     * 数字页的天然用途是算账、填表、写日期，所以上滑给的是 + − × ÷ = % ：按数字
     * 时手指正好在那儿，滑一下就出运算符，不用切到符号页再切回来。
     *
     * `0` 的上滑给 `：`（时间/比例写法）、`，`/`。` 两键上滑给 `；`/`～`——
     * 它们本来就在底部显示中文标点，上滑换成同族的下一个最顺。
     *
     * **不做 `(` `)` 的上滑**：它们在字母页 `k` 上有，而数字页这两格要留给
     * 「小数点和正负号之外更常用的东西」——实际填表时 `(` 出现频率远低于 `%`。
     */
    val SWIPE: Map<String, String> = mapOf(
        "1" to "+", "2" to "-", "3" to "×",
        "4" to "÷", "5" to "=", "6" to "%",
        "7" to "(", "8" to ")", "9" to "/",
        "0" to "：",
    )

    private fun txt(s: String) = Key(s, KeyKind.TEXT, s, weight = 2f, swipe = SWIPE[s])

    /**
     * 数字页**不分中英**，底部两角一律是半角 `,` 和 `.`。
     *
     * ## 为什么中文模式也不用全角
     *
     * 以前中文页是 `，0。`。但数字页的用途是算账、填表、写金额、输电话和版本号——
     * 这些场景下游（Excel、银行 App、表单校验、命令行）**一律不认全角标点**：
     * `12。5` 不是小数，`1，000` 不是千分位，而是两个多余字符。
     *
     * 更要紧的是**小数点**：中文键盘上原本根本没有半角 `.`，付钱时输 `12.5` 得切
     * 到英文页再切回来。这已经不是「不方便」而是「这条路走不通」。
     *
     * 全角标点在中文文本里当然该用，但它属于**文字输入**；数字页是**数字输入**。
     * 两个场景的标点规则本来就不同，不该共用一套。
     */
    private val ROWS: List<List<Key>> = listOf(
        listOf("1", "2", "3").map(::txt),
        listOf("4", "5", "6").map(::txt),
        listOf("7", "8", "9").map(::txt),
        listOf(
            // `.` 就是小数点：键面写 `.` 而不是「点」，按下去直接上屏半角句点
            Key(",", KeyKind.TEXT, ",", weight = 2f, swipe = ";"),
            Key("0", KeyKind.TEXT, "0", weight = 2f, swipe = ":"),
            Key(".", KeyKind.TEXT, ".", weight = 2f, swipe = "~"),
        ),
        // 功能排：和数字行一样按「总权重 6」分，否则这一排的键宽会和上面跳一格。
        // `符` 去符号页，`ABC` 回字母页，`123` 在符号页上用来回数字页。
        compactFunctionRow(listOf(NAV_LETTERS, NAV_SYMBOLS, NAV_EMOJI)),
    )

    val rows: List<List<Key>> get() = ROWS

    /** 数字页中英一致。保留这个签名是为了和其它页的调用点统一。 */
    fun rowsOf(chinese: Boolean): List<List<Key>> = ROWS

    /** 所有直接上屏的字符。测试用：不许有重复，也不许混进字母。 */
    val textCodes: List<String>
        get() = rows.flatten().filter { it.kind == KeyKind.TEXT }.map { it.code }

    /** 所有上滑字符。测试用：不许和主字符撞车，也不许重复。 */
    val swipeCodes: List<String>
        get() = rows.flatten().mapNotNull { it.swipe }
}

/**
 * 中文标点 / 常用符号页。形状和数字页完全一样（5 行、每行总权重 6），
 * 这样两页之间切换时**每一个键的位置都不动**，肌肉记忆能直接复用。
 */
object SymbolLayout {

    /**
     * 符号键的上滑字符：**同族的下一个**。
     *
     * 符号页的每一格已经放了三列最常用的标点，上滑补的是「同一个键位上、稍微少用
     * 一点但一旦要用就必须找到」的那些：书名号、方括号、花括号、斜杠反斜杠、引号。
     *
     * 为什么不直接把它们铺成第 6 行：紧凑页的高度是按「5 行 × 36dp ≈ 字母页 4 行」
     * 算死的（见 [NumericLayout] 的注释），加一行会让切页时输入框往上跳。
     */
    val SWIPE: Map<String, String> = mapOf(
        "，" to "<", "。" to ">", "、" to "|",
        "！" to "“", "？" to "”", "：" to "‘",
        "；" to "’", "（" to "《", "）" to "》",
        "…" to "[", "—" to "]", "·" to "\\",
    )

    private fun txt(s: String) = Key(s, KeyKind.TEXT, s, weight = 2f, swipe = SWIPE[s])

    /** 英文符号页：半角标点。以前整张符号页全是全角，英文输入找不到句号/逗号。 */
    private val EN_SWIPE: Map<String, String> = mapOf(
        "," to "<", "." to ">", ";" to "|", "!" to "\"",
        "?" to "'", ":" to "~", "'" to "`", "\"" to "\\",
        "(" to "{", ")" to "}", "[" to "<", "]" to ">",
        "-" to "_", "_" to "=", "/" to "|", "+" to "*",
    )

    private val CHINESE_ROWS: List<List<Key>> = listOf(
        // 顿号跟逗号、句号放一排：它们是同一类「断句」标点，列举报菜名用得最多。
        // 以前它夹在「！：」中间，找的时候要逐格扫一遍。
        listOf("，", "。", "、").map(::txt),
        listOf("！", "？", "：").map(::txt),
        listOf("；", "（", "）").map(::txt),
        listOf("…", "—", "·").map(::txt),
        compactFunctionRow(listOf(NAV_LETTERS, NAV_DIGITS, NAV_EMOJI)),
    )

    /**
     * 英文符号页。
     *
     * 结构照抄中文页（5 行、每行 3 个、最后一行功能键）——**行数和行高必须一致**，
     * 否则中英切换时键盘高度会跳，输入框跟着上下蹿。只把内容换成半角。
     *
     * 这里必须有 `.` 和 `,`：用户反馈英文模式下整个键盘（连通符号页）找不到句号，
     * QQ 邮箱地址写不完。字母页上滑也补了半角句号，两条路都能打到。
     */
    private val ENGLISH_ROWS: List<List<Key>> = listOf(
        listOf(",", ".", ";").map { Key(it, KeyKind.TEXT, it, weight = 2f, swipe = EN_SWIPE[it]) },
        listOf("!", "?", ":").map { Key(it, KeyKind.TEXT, it, weight = 2f, swipe = EN_SWIPE[it]) },
        listOf("'", "(", ")").map { Key(it, KeyKind.TEXT, it, weight = 2f, swipe = EN_SWIPE[it]) },
        listOf("\"", "-", "/").map { Key(it, KeyKind.TEXT, it, weight = 2f, swipe = EN_SWIPE[it]) },
        compactFunctionRow(listOf(NAV_LETTERS, NAV_DIGITS, NAV_EMOJI)),
    )

    val rows: List<List<Key>> get() = CHINESE_ROWS

    /** 按中英模式取符号页行数据。 */
    fun rowsOf(chinese: Boolean): List<List<Key>> =
        if (chinese) CHINESE_ROWS else ENGLISH_ROWS

    val textCodes: List<String>
        get() = rows.flatten().filter { it.kind == KeyKind.TEXT }.map { it.code }

    val swipeCodes: List<String>
        get() = rows.flatten().mapNotNull { it.swipe }
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

/**
 * 光标与编辑控制页。
 *
 * ## 为什么方向键要 4 个独立格子 + 中间留空
 *
 * 十字布局（上 / 左-右 / 下）的唯一价值就是**拇指不用看也能找到中间那颗**。做成
 * 一行四个 `← ↑ ↓ →` 看着整齐，实际每次都得扫一眼确认方向——比直接点文本框还慢。
 * 所以这里按十字摆：第 2 行 `(空) ↑ (空)`，第 3 行 `← ↓ →`。
 *
 * ## 为什么复制/粘贴不合并成一个「剪贴板」键
 *
 * 手机上复制和粘贴是两个完全不同的时刻（复制时人还在原文里，粘贴时人已经跳到
 * 目标位置）。合并成一个键就得先判断"现在该复制还是该粘贴"，而输入法**猜不准**：
 * 有选区就复制这个规则在聊天框里经常把用户的草稿覆盖掉。所以两个键各占一格。
 *
 * ## 权重
 *
 * 3 列，每格 2 → 合计 6，和其它紧凑页完全一致（换页时键宽不错位）。
 * 第 1、3 行的空位用**真实的空键**占住而不是少放一个：少放会让 `↑` 被拉成整行宽。
 */
object NavLayout {

    private fun nav(label: String, action: NavAction) =
        Key(label, KeyKind.NAV, action.name, weight = 2f)

    private fun blank() = Key("", KeyKind.TEXT, "", weight = 2f)

    val rows: List<List<Key>> = listOf(
        listOf(nav("全选", NavAction.SELECT_ALL), nav("复制", NavAction.COPY), nav("粘贴", NavAction.PASTE)),
        listOf(blank(), nav("↑", NavAction.UP), blank()),
        listOf(nav("←", NavAction.LEFT), nav("↓", NavAction.DOWN), nav("→", NavAction.RIGHT)),
        listOf(nav("剪切", NavAction.CUT), nav("行首", NavAction.LINE_START), nav("行尾", NavAction.LINE_END)),
        compactFunctionRow(listOf(NAV_LETTERS, NAV_DIGITS, NAV_SYMBOLS)),
    )
}

/** 给壳用的一张小表：页 → 行数据 + 行高。避免 FeiheIme 里散落三个 when。 */
object KeyboardPages {
    fun rows(page: Page): List<List<Key>> = rows(page, chinese = true)

    /**
     * 按中英模式取行数据。
     *
     * 只有字母页/数字页/符号页区分中英——表情和控制页没有标点，不参与。
     * 中英切换时壳会调 `showPage` 重建键盘，所以这里只要保证"同一模式同一份数据"。
     */
    fun rows(page: Page, chinese: Boolean): List<List<Key>> = when (page) {
        Page.LETTERS -> QwertyLayout.rowsOf(chinese)
        Page.DIGITS -> NumericLayout.rowsOf(chinese)
        Page.SYMBOLS -> SymbolLayout.rowsOf(chinese)
        Page.EMOJI -> EmojiLayout.rows
        Page.NAV -> NavLayout.rows
    }

    /** 见 [NumericLayout] 里的高度计算：紧凑页 36dp，总和与字母页 46dp×4 相当。 */
    fun rowHeightDp(page: Page): Float = if (page == Page.LETTERS) 46f else 36f

    fun label(page: Page): String = when (page) {
        Page.LETTERS -> "26 键"
        Page.DIGITS -> "数字"
        Page.SYMBOLS -> "符号"
        Page.EMOJI -> "表情"
        Page.NAV -> "控制"
    }
}
