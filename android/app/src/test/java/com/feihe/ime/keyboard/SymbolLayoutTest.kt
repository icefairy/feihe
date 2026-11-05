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
        for (p in listOf(Page.DIGITS, Page.SYMBOLS)) {
            val h = 5 * KeyboardPages.rowHeightDp(p) + 4 * gap + padding
            assertTrue(
                "${p} 页高 $h 与字母页 $letters 差太多，切页会跳",
                kotlin.math.abs(h - letters) <= 4f,
            )
        }
    }

    @Test
    fun `三页的行数据和页名都能取到`() {
        for (p in Page.entries) {
            assertTrue("${p} 没有行数据", KeyboardPages.rows(p).isNotEmpty())
            assertTrue("${p} 没有页名", KeyboardPages.label(p).isNotEmpty())
        }
        assertEquals(QwertyLayout.rows, KeyboardPages.rows(Page.LETTERS))
        assertEquals(NumericLayout.rows, KeyboardPages.rows(Page.DIGITS))
        assertEquals(SymbolLayout.rows, KeyboardPages.rows(Page.SYMBOLS))
    }
}
