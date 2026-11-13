package com.feihe.ime.keyboard

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 键位表的规格锁。
 *
 * 这些断言防的不是"写错"，是**改漏**：小鹤不改键位，但一旦有人为了"把韵母印到键上"
 * 而动这张表，26 个字母就会少一个或重一个——那种错误在真机上表现为"某个音永远打不出来"，
 * 极难定位。
 */
class QwertyLayoutTest {

    @Test
    fun `26 个字母一个不多一个不少`() {
        val codes = QwertyLayout.letterCodes
        assertEquals(26, codes.size)
        assertEquals(('a'..'z').toList(), codes.map { it[0] }.sorted())
    }

    @Test
    fun `字母键的 code 都是单个小写字母`() {
        for (k in QwertyLayout.rows.flatten().filter { it.kind == KeyKind.CHAR }) {
            assertEquals("键 ${k.label} 的 code 不对", 1, k.code.length)
            assertTrue("键 ${k.label} 的 code 必须小写", k.code[0] in 'a'..'z')
        }
    }

    @Test
    fun `没有重复的字母`() {
        val codes = QwertyLayout.letterCodes
        assertEquals(codes.size, codes.toSet().size)
    }

    @Test
    fun `四排，每排不超过 10 键`() {
        assertEquals(4, QwertyLayout.rows.size)
        for ((i, row) in QwertyLayout.rows.withIndex()) {
            assertTrue("第 ${i + 1} 排有 ${row.size} 个键，太挤了", row.size <= 10)
        }
    }

    @Test
    fun `空格是最宽的键`() {
        val space = QwertyLayout.rows.flatten().first { it.kind == KeyKind.SPACE }
        val widest = QwertyLayout.rows.flatten().maxOf { it.weight }
        assertEquals("空格必须是最宽的键，否则误触率会很高", widest, space.weight, 0.001f)
    }

    @Test
    fun `必须有的功能键都在`() {
        val kinds = QwertyLayout.rows.flatten().map { it.kind }.toSet()
        for (need in listOf(KeyKind.DELETE, KeyKind.SPACE, KeyKind.ENTER, KeyKind.MODE)) {
            assertTrue("缺功能键 $need", need in kinds)
        }
    }

    /**
     * 字母页上**不许再有独立的标点键**（2026-11-12 用户要求）。
     *
     * 原来空格两边是「，、␣。」，拇指盲按空格时经常蹭到标点、误上屏一个中文逗号。
     * 现在这些全部收进上滑（c/m/v），底排只剩功能键和空格。
     */
    @Test
    fun `字母页有且只有一个独立标点键，默认逗号上滑句号`() {
        // 规格变更（用户 2026-11-13）：以前逗号句号全收进 c/v 上滑，用户反馈
        // 「每次打标点都要上滑」太重，要求在 123 和 😊 之间加一个独立标点键，
        // 默认逗号、上滑句号。
        val puncts = QwertyLayout.rows.flatten().filter { it.kind == KeyKind.PUNCT }
        assertEquals("标点键应该正好一个，实际：${puncts.map { it.label }}", 1, puncts.size)
        val p = puncts.first()
        assertEquals("默认上屏应该是逗号", "，", p.code)
        assertEquals("上滑应该是句号", "。", p.swipe)

        // 位置：紧挨在 123 右边
        val bottom = QwertyLayout.rows.last()
        val pageIdx = bottom.indexOfFirst { it.kind == KeyKind.PAGE && it.code == "digits" }
        assertEquals("标点键必须紧跟在 123 右边一位", p, bottom[pageIdx + 1])
    }

    /** 标点键必须用 PUNCT：它要先把候选上屏再粘标点，不能用 TEXT 直接怼。 */
    @Test
    fun `标点键是 PUNCT 型 —— 打拼音时点它得到汉字加标点`() {
        val p = QwertyLayout.rows.flatten().first { it.kind == KeyKind.PUNCT }
        assertEquals(KeyKind.PUNCT, p.kind)
    }

    /** 英文态标点键必须是半角：全角逗号进邮箱/命令行都是非法字符。 */
    @Test
    fun `英文态标点键是半角逗号上滑句号`() {
        val p = QwertyLayout.rowsOf(chinese = false).flatten().first { it.kind == KeyKind.PUNCT }
        assertEquals(",", p.code)
        assertEquals(".", p.swipe)
    }

    /** 空格两侧必须都是功能键：不许是打字键或标点，否则盲按空格会误上屏。 */
    @Test
    fun `空格两侧只留功能键`() {
        val bottom = QwertyLayout.rows.last()
        val spaceIdx = bottom.indexOfFirst { it.kind == KeyKind.SPACE }
        assertTrue("底排必须有空格", spaceIdx > 0)
        // 左边必须是功能键：以前这里断言的是 PAGE，但「中」是 MODE——
        // 它同样是功能键（不产生字符），旧断言写窄了。真正要防的是标点/字母。
        val left = bottom[spaceIdx - 1]
        assertTrue(
            "空格左边是「${left.label}」，不该是打字键或标点",
            left.kind in listOf(KeyKind.PAGE, KeyKind.MODE, KeyKind.DELETE),
        )
        val right = bottom.getOrNull(spaceIdx + 1)
        assertTrue(
            "空格右边是「${right?.label}」，不该是打字键或标点",
            // MODE（中/英）是功能键，不产生字符。原来这里漏了它——
            // 参照豆包把「中」挪到空格右边后，这条断言就误报了。
            right == null || right.kind in
                listOf(KeyKind.PAGE, KeyKind.ENTER, KeyKind.DELETE, KeyKind.MODE),
        )
    }

    /** 斜杠以前整个键盘都打不出来，现在必须在某个字母键的上滑里。 */
    @Test
    fun `字母页能上滑打出斜杠`() {
        val swipes = QwertyLayout.rows.flatten().mapNotNull { it.swipe }
        assertTrue("字母页上滑里必须有 /（输入网址、路径、日期都要它）", "/" in swipes)
    }

    /** 中文断句四件套都要能在字母页上滑打出来。 */
    @Test
    fun `上滑能打出中文标点（逗号句号改由独立键承担）`() {
        val rows = QwertyLayout.rows
        val swipes = rows.flatten().mapNotNull { it.swipe }
        // 逗号句号已经从 c/v 上滑移到独立标点键上，这里只查剩下的
        for (p in listOf("！", "？", "：", "；", "、")) {
            assertTrue("上滑里缺「$p」", p in swipes)
        }
        // c/v 上滑让位给书名号，两个都要能打出来
        for (p in listOf("《", "》")) {
            assertTrue("上滑里缺「$p」", p in swipes)
        }
        // 逗号句号仍然可达，只是换了位置：独立键本体 + 上滑
        val punct = rows.flatten().first { it.kind == KeyKind.PUNCT }
        assertEquals("，", punct.code)
        assertEquals("。", punct.swipe)
    }

    /**
     * **英文模式的上滑里不能有一丝中文标点。**
     *
     * 这条是被用户实测抓出来的：切到英文后第二排 K/L 还是弯引号 `“ ”`，
     * 而且整个键盘没有直引号 `'` `"`——写代码、填 JSON、发命令行时
     * 弯引号一律是非法字符，用户只能切回中文或进符号页。
     *
     * 白名单式断言（不是黑名单）：把英文表里允许出现的字符全列出来，
     * 多一个就报错。这样以后有人往表里加中文标点会被当场拦住，
     * 而不是等用户用到才发现。
     */
    @Test
    fun `英文上滑只有半角符号`() {
        // 白名单 = 全部 ASCII 可见符号（可打印标点区）+ 省略号。
        // 以前这里手写枚举，加个 `$`/`*`/`=` 就被误判成"非半角"——
        // 真正的判据是「字符码位在 ASCII 标点范围内」，不是「我记得的那些符号」。
        val allowed = (33..126).map { it.toChar() }.joinToString("") + "…"
        val bad = QwertyLayout.swipeMap(chinese = false).entries
            .filter { it.value.any { c -> c !in allowed } }
            .map { "${it.key}→${it.value}" }
        assertTrue("英文上滑里混进了非半角字符：$bad", bad.isEmpty())
    }

    /**
     * **英文态不许丢符号。**
     *
     * 这条是被真改坏过才加的：重排上滑表时把 `…` 挤掉了，而当时的断言只查
     * "有没有混进全角"，不查"该有的还在"，测试全绿但用户打不出省略号。
     *
     * 判据：字母页上滑 ∪ 底排标点键（本体+上滑）∪ 符号页，必须覆盖这组基准符号。
     * 加新符号不加进来没关系，**但不能让已经能打的消失**。
     */
    @Test
    fun `英文态基准符号一个都不能少`() {
        val en = QwertyLayout.rowsOf(chinese = false).flatten()
        val reachable = HashSet<String>()
        en.forEach { k -> k.swipe?.let { reachable.add(it) } }
        en.filter { it.kind == KeyKind.TEXT || it.kind == KeyKind.PUNCT }
            .forEach { reachable.add(it.code) }
        SymbolLayout.rowsOf(chinese = false).flatten()
            .filter { it.kind == KeyKind.TEXT }.forEach { reachable.add(it.code) }
        // 0.3.79 能打出的基准集合，只许增不许减
        val baseline = "!\"#'(),-./:;?@~…".toList()
        for (c in baseline) {
            assertTrue("英文态打不出「$c」了", c.toString() in reachable)
        }
    }

    /** 英文模式必须能直接打出单引号和双引号（代码/JSON/命令行刚需）。 */
    @Test
    fun `英文上滑有单双引号`() {
        val en = QwertyLayout.swipeMap(chinese = false).values
        assertTrue("英文上滑缺单引号 '", "'" in en)
        assertTrue("英文上滑缺双引号 \"", "\"" in en)
    }

    /** 中文模式仍要保留弯引号——中文排版里直引号是错的。 */
    @Test
    fun `中文上滑保留中文双引号`() {
        val zh = QwertyLayout.swipeMap(chinese = true).values
        assertTrue("中文上滑缺左弯引号 “", "“" in zh)
        assertTrue("中文上滑缺右弯引号 ”", "”" in zh)
    }

    /**
     * 每个字母键都要有上滑字符，且互不重复。
     *
     * 少了某一个 → 那个键上滑没反应（用户会以为手势时灵时不灵）；重复 → 两个键
     * 上滑出同一个字符，等于其中一个是多余的。
     */
    @Test
    fun `每个字母键都有上滑字符`() {
        val letterKeys = QwertyLayout.rows.flatten().filter { it.kind == KeyKind.CHAR }
        for (k in letterKeys) {
            assertTrue("键 ${k.label} 没有上滑字符", k.swipe != null)
        }
        val swipes = letterKeys.mapNotNull { it.swipe }
        assertEquals("上滑字符不能重复", swipes.size, swipes.toSet().size)
    }

    /** 字母页必须能直接进表情页：聊天时八成时间都在字母页。 */
    @Test
    fun `字母页有表情入口`() {
        val emoji = QwertyLayout.rows.flatten().firstOrNull {
            it.kind == KeyKind.PAGE && it.code == "emoji"
        }
        assertTrue("字母页缺 😊 入口", emoji != null)
    }

    /**
     * 字母页上没有任何地方能打数字，`123` 是唯一入口。
     *
     * 少了它不会崩，但用户会认为「这个输入法打不了数字」——而且找不到任何补救办法。
     */
    @Test
    fun `有数字页入口`() {
        val target = QwertyLayout.rows.flatten().firstOrNull { it.kind == KeyKind.PAGE }
        assertTrue("字母页必须有 123 键", target != null)
        assertEquals("123", target!!.label)
        assertEquals("digits", target.code)
    }

    // ---- 中英模式标点分离 + 空格居中（2026-11-12 用户要求）----

    /**
     * 英文模式下上滑必须出**半角**标点。
     *
     * 用户反馈：切成英文后上滑还是「！？，。：；、」，QQ 邮箱地址根本写不完。
     * 全角标点在下游（邮箱、网址、命令行）一律算非法字符。
     */
    @Test
    fun `英文模式上滑出半角标点`() {
        val en = QwertyLayout.rowsOf(chinese = false).flatten().mapNotNull { it.swipe }
        // 逗号句号改由底排独立标点键承担（本体 , / 上滑 .），不在字母上滑里
        for (p in listOf("!", "?", ":", ";", "-", "/", "'", "\"")) {
            assertTrue("英文模式上滑缺半角「$p」，实际：${en.filter { it.length == 1 }}", p in en)
        }
        // 逗号句号仍然可达：底排独立键
        val punct = QwertyLayout.rowsOf(chinese = false).flatten().first { it.kind == KeyKind.PUNCT }
        assertEquals(",", punct.code)
        assertEquals(".", punct.swipe)
    }

    /** 英文模式字母页上滑里**不许**再出现全角中文标点。 */
    @Test
    fun `英文模式上滑没有全角标点`() {
        val en = QwertyLayout.rowsOf(chinese = false).flatten().mapNotNull { it.swipe }
        val fullwidth = listOf("！", "？", "，", "。", "：", "；", "、")
        for (p in fullwidth) {
            assertTrue("英文模式上滑不该有全角「$p」", p !in en)
        }
    }

    /** 中文模式仍然出全角，两套互不污染。 */
    @Test
    fun `中文模式上滑仍是全角`() {
        val zh = QwertyLayout.rowsOf(chinese = true).flatten().mapNotNull { it.swipe }
        // 逗号句号现在挂在独立标点键上（本体 ， / 上滑 。），见专门的用例
        for (p in listOf("！", "？", "：", "；", "、", "《", "》")) {
            assertTrue("中文模式上滑缺全角「$p」", p in zh)
        }
        // 半角句号不该在中文模式里（邮箱是英文场景才要）
        assertTrue("中文模式不该有半角句号", "." !in zh)
    }

    /**
     * 空格要居中，但**允许偏左一点**。
     *
     * 历史：这里原来断言左右权重**严格相等**。那是「底排只有 123/中/空格/😊/⌘/↵
     * 且回车=1.0」时的配平方式。
     *
     * 现在参照豆包输入法改了两处，两个改动都把权重推向右边：
     * - 回车从 1.0 提到 1.8（用户明确要求，原来太窄按不中）；
     * - 「中」从空格左边挪到右边（豆包/iOS 的放法）。
     *
     * 右边自然比左边重，空格中心落在 44% 左右。豆包自己也是 46%，
     * 实测没人觉得歪。硬要相等就得把回车压回 1.0，那是退步。
     *
     * 所以断言改成「偏左但不超过一个键宽」：中心在 40%~50%。
     * 这条仍然能抓住真正的错配（比如权重写反、回车又变窄）。
     */
    @Test
    fun `空格居中 但允许参照豆包略偏左`() {
        val bottom = QwertyLayout.rows.last()
        val spaceIdx = bottom.indexOfFirst { it.kind == KeyKind.SPACE }
        assertTrue("底排必须有空格", spaceIdx >= 0)
        val left = bottom.take(spaceIdx).sumOf { it.weight.toDouble() }
        val right = bottom.drop(spaceIdx + 1).sumOf { it.weight.toDouble() }
        val total = left + right + bottom[spaceIdx].weight
        val centerPct = (left + bottom[spaceIdx].weight / 2) / total * 100

        assertTrue(
            "空格中心应在 40%~50%（偏左不超过 10%），实际 ${"%.1f".format(centerPct)}%",
            centerPct in 40.0..50.0,
        )
        // 差值不能太大：超过两个键宽就是布局真的失衡了
        assertTrue(
            "左右权重差 ${right - left} 过大",
            kotlin.math.abs(right - left) <= 1.5,
        )
    }

    /**
     * 底排总权重必须等于 10，和第一排对齐。
     *
     * 各排是独立按自己的总和分配键宽的，所以不要求四排相等（第二三排是 9）。
     * 但底排改过之后要盯住这个数：它一变，底排每个键的宽度都会跟着变，
     * 空格居中也会被破坏。
     */
    @Test
    fun `底排总权重和字母排一致`() {
        val bottom = QwertyLayout.rows.last()
        val sum = bottom.sumOf { it.weight.toDouble() }
        // 原值 10.0 是「底排 6 个键」时的配平。用户要求加一个独立标点键后
        // 变成 7 个键：
        //   123(1.0) + ，(1.0) + 😊(0.7) + 空格(3.9) + 中(1.0) + ⌘(0.7) + ↵(1.8) = 10.1
        //
        // 为什么是 10.1 而不是回到 10.0：把空格再压 0.1 会让它窄 2.5%，
        // 而 10.1 与第一排（10 个字母，总权重 10.0）只差 1%——键宽视觉上看不出，
        // 又不牺牲空格手感。硬凑整数是形式主义。
        assertEquals("底排总权重应≈10（字母排 10.0）", 10.1, sum, 0.001)
    }

    /**
     * 回车必须比普通功能键宽（用户要求：原来太窄按不中）。
     *
     * 1.5 是「明显比别人宽一档」的最小值：底排单位键宽是 total/6≈1.67，
     * 回车 1.5 已接近均分宽度，而 1.0 只有它的三分之二。
     */
    @Test
    fun `回车比普通功能键宽`() {
        val bottom = QwertyLayout.rows.last()
        val enter = bottom.first { it.kind == KeyKind.ENTER }
        // 1.8 是照豆包（换行键≈1.76 个字母键宽）定的。
        // 这里只断言「明显比别人宽」，不钉死具体值——以后微调 1.7/1.9 不该挂测试。
        assertTrue("回车权重 ${enter.weight} 应 ≥1.7（豆包 1.76）", enter.weight >= 1.7f)
        // 比 123/中 之外的窄键都要宽
        val narrow = bottom.filter { it.kind == KeyKind.PAGE && it.code == "emoji" }
        assertTrue(
            "回车必须比 😊 宽，否则还是按不中",
            enter.weight > narrow.first().weight,
        )
    }

    /**
     * 退格和 ⇧ 权重相等：第三排两端对称。
     *
     * 曾经把退格压到 1（想让它和回车对调宽度），结果第三排合计变 9.5，
     * Z 排字母键比 Q 排窄 1.5%，换页时键宽会跳。这条守着别再犯。
     */
    @Test
    fun `退格和上档键等宽 第三排才对称`() {
        val third = QwertyLayout.rows[2]
        val shift = third.first { it.kind == KeyKind.SHIFT }
        val del = third.first { it.kind == KeyKind.DELETE }
        assertEquals("退格与 ⇧ 必须等宽", shift.weight, del.weight, 0.001f)
        val sum = third.sumOf { it.weight.toDouble() }
        assertEquals("第三排合计必须是 10，否则字母键宽和第一排不一致", 10.0, sum, 0.001)
    }
}
