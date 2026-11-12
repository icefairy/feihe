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
    fun `字母页没有独立标点键`() {
        val puncts = QwertyLayout.rows.flatten().filter { it.kind == KeyKind.PUNCT }
        assertTrue("字母页不该再有独立的标点键，实际有：${puncts.map { it.label }}", puncts.isEmpty())
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
            right == null || right.kind in listOf(KeyKind.PAGE, KeyKind.ENTER, KeyKind.DELETE),
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
    fun `上滑能打出中文断句标点`() {
        val swipes = QwertyLayout.rows.flatten().mapNotNull { it.swipe }
        for (p in listOf("，", "。", "！", "？", "：", "；", "、")) {
            assertTrue("上滑里缺「$p」", p in swipes)
        }
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
        for (p in listOf("!", "?", ",", ".", ":", ";")) {
            assertTrue("英文模式上滑缺半角「$p」，实际：${en.filter { it.length == 1 }}", p in en)
        }
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
        for (p in listOf("！", "？", "，", "。")) {
            assertTrue("中文模式上滑缺全角「$p」", p in zh)
        }
        // 半角句号不该在中文模式里（邮箱是英文场景才要）
        assertTrue("中文模式不该有半角句号", "." !in zh)
    }

    /**
     * 底排空格必须**居中**。
     *
     * 用户反馈「空格太偏右」。原来左边 123/中/😊/⌘ 共 3.6，右边只有回车 1.2，
     * 空格被挤到右边。现在左右各 2.6，用权重相等来保证视觉居中——
     * 权重就是实际宽度，所以这条能直接测出来。
     */
    @Test
    fun `空格左右权重相等所以居中`() {
        val bottom = QwertyLayout.rows.last()
        val spaceIdx = bottom.indexOfFirst { it.kind == KeyKind.SPACE }
        assertTrue("底排必须有空格", spaceIdx >= 0)
        val left = bottom.take(spaceIdx).sumOf { it.weight.toDouble() }
        val right = bottom.drop(spaceIdx + 1).sumOf { it.weight.toDouble() }
        assertEquals("空格左边权重 $left 与右边 $right 必须相等才居中", left, right, 0.001)
    }

    /**
     * 底排总权重必须等于 10，和第一排对齐。
     *
     * 各排是独立按自己的总和分配键宽的，所以不要求四排相等（第二三排是 9）。
     * 但底排改过之后要盯住这个数：它一变，底排每个键的宽度都会跟着变，
     * 空格居中也会被破坏。
     */
    @Test
    fun `底排总权重不变`() {
        val bottom = QwertyLayout.rows.last()
        val sum = bottom.sumOf { it.weight.toDouble() }
        assertEquals("底排总权重变了会让每个键宽度都变", 10.0, sum, 0.001)
    }
}
