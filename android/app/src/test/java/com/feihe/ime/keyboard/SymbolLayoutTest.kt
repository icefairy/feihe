package com.feihe.ime.keyboard

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 符号页 + 三页总表的规格锁。
 *
 * 最要紧的一条是**高度**：字母页 4 排 46dp、紧凑页 5 排 36dp，加起来几乎一样。
 * 哪天有人把紧凑页行高改成 46dp，切页时输入框会往上跳 50dp，而这类「感觉不太对」
 * 的回归在真机上很难描述、更难定位。
 */
class SymbolLayoutTest {

    @Test
    fun `五行且各行权重合计与数字页一致`() {
        val totals = SymbolLayout.rows.map { row -> row.sumOf { it.weight.toDouble() } }
        assertEquals(5, totals.size)
        for ((i, t) in totals.withIndex()) {
            assertEquals("第 ${i + 1} 行权重合计应是 6", 6.0, t, 0.001)
        }
        // 两页的键位网格必须一模一样，切页时手指不用重新找位置
        assertEquals(
            NumericLayout.rows.map { it.size },
            SymbolLayout.rows.map { it.size },
        )
    }

    @Test
    fun `常用中文标点都在`() {
        val codes = SymbolLayout.textCodes.toSet()
        for (p in listOf("，", "。", "？", "！", "：", "、")) {
            assertTrue("缺标点 $p", p in codes)
        }
    }

    @Test
    fun `有回字母页和回数字页的键`() {
        val pages = SymbolLayout.rows.flatten().filter { it.kind == KeyKind.PAGE }
        assertTrue("要有回字母页的 ABC", pages.any { it.label == "ABC" && it.code == "letters" })
        assertTrue("要有回数字页的 123", pages.any { it.label == "123" && it.code == "digits" })
    }

    @Test
    fun `符号里没有字母也不重复`() {
        for (c in SymbolLayout.textCodes) {
            assertFalse("「$c」是字母", c.any { it in 'a'..'z' || it in 'A'..'Z' })
            assertTrue("「$c」是空串", c.isNotEmpty())
        }
        val codes = SymbolLayout.textCodes
        assertEquals("符号页上有重复：$codes", codes.size, codes.toSet().size)
    }

    @Test
    fun `紧凑页和字母页的总高度相当`() {
        val gap = 5f
        val padding = 12f
        val letters = 4 * KeyboardPages.rowHeightDp(Page.LETTERS) + 3 * gap + padding
        for (p in listOf(Page.DIGITS, Page.SYMBOLS, Page.EMOJI)) {
            val h = 5 * KeyboardPages.rowHeightDp(p) + 4 * gap + padding
            assertTrue(
                "${p} 页高 $h 与字母页 $letters 差太多，切页会跳",
                kotlin.math.abs(h - letters) <= 4f,
            )
        }
    }

    @Test
    fun `每一页的行数据和页名都能取到`() {
        for (p in Page.entries) {
            assertTrue("${p} 没有行数据", KeyboardPages.rows(p).isNotEmpty())
            assertTrue("${p} 没有页名", KeyboardPages.label(p).isNotEmpty())
        }
        assertEquals(QwertyLayout.rows, KeyboardPages.rows(Page.LETTERS))
        assertEquals(NumericLayout.rows, KeyboardPages.rows(Page.DIGITS))
        assertEquals(SymbolLayout.rows, KeyboardPages.rows(Page.SYMBOLS))
        assertEquals(EmojiLayout.rows, KeyboardPages.rows(Page.EMOJI))
    }

    /**
     * 顿号必须在**字母页和符号页都点得到**。
     *
     * 它是中文里最常用的标点之一（列举报菜名、地名），藏进二级页面等于没有；
     * 用户报过「符号页找不到顿号」，所以这条钉死——两页各有一个、且都能直接上屏。
     */
    @Test
    fun `顿号在符号页有实体键，在字母页走上滑`() {
        // 2026-11-12 改：字母页的独立顿号键被去掉了（用户要求空格旁不留标点），
        // 但「顿号必须一按就到」这条不许退——它现在在 m 键的上滑里。
        val letters = QwertyLayout.rows.flatten().filter { it.label == "、" }
        assertEquals("字母页不该再有实体顿号键", 0, letters.size)
        assertTrue(
            "字母页必须还能打出顿号（m 键上滑）",
            QwertyLayout.rows.flatten().any { it.swipe == "、" },
        )
        val symbols = SymbolLayout.rows.flatten().filter { it.label == "、" }
        assertEquals("符号页应有一个顿号键", 1, symbols.size)
        assertEquals(KeyKind.TEXT, symbols[0].kind)
    }

    // ---- 英文符号页（2026-11-12 用户要求）----

    /**
     * 英文符号页**必须有半角句号和逗号**。
     *
     * 用户反馈：英文模式下整个键盘连通符号页都找不到句号，QQ 邮箱地址无法完成。
     * 这条是那次反馈的直接规格锁。
     */
    @Test
    fun `英文符号页有半角句号逗号`() {
        val labels = SymbolLayout.rowsOf(chinese = false).flatten()
            .filter { it.kind == KeyKind.TEXT }
            .map { it.code }
        assertTrue("英文符号页必须有半角句号 .", "." in labels)
        assertTrue("英文符号页必须有半角逗号 ,", "," in labels)
    }

    /** 英文符号页不许再混进全角标点。 */
    @Test
    fun `英文符号页没有全角标点`() {
        val labels = SymbolLayout.rowsOf(chinese = false).flatten()
            .filter { it.kind == KeyKind.TEXT }
            .map { it.code }
        for (p in listOf("，", "。", "、", "！", "？", "：", "；")) {
            assertTrue("英文符号页不该有全角「$p」", p !in labels)
        }
    }

    /**
     * 中英两套符号页的**行数和行高必须一致**。
     *
     * 不一致的话，中英切换时键盘总高度会跳，输入框跟着上下蹿——这类回归在真机上
     * 表现为"打字时整个聊天窗口抖一下"，比崩溃还难描述。
     */
    @Test
    fun `中英符号页行数一致`() {
        assertEquals(
            "中英符号页行数必须相同，否则切模式时键盘高度会跳",
            SymbolLayout.rowsOf(chinese = true).size,
            SymbolLayout.rowsOf(chinese = false).size,
        )
    }
}
