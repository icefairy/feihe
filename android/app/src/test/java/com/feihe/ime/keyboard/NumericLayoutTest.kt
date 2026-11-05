package com.feihe.ime.keyboard

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 数字页的规格锁。
 *
 * 这一页最怕的不是「少一个符号」，而是**缺数字**——字母页上没有任何地方能打
 * `0`-`9`，这一页是唯一入口，少一个就是永久打不出来。
 *
 * 第二组断言锁的是「3×3」这个形状本身：改成三列就是为了把键宽从 1/5 提到 1/3，
 * 哪天有人为了塞更多符号又加回 5 列，这里会先红。
 */
class NumericLayoutTest {

    @Test
    fun `数字 0 到 9 一个不少`() {
        val codes = NumericLayout.textCodes.toSet()
        for (d in '0'..'9') {
            assertTrue("缺数字 $d", d.toString() in codes)
        }
    }

    @Test
    fun `1 到 9 是三行三列的九宫格`() {
        for ((row, expect) in listOf("123", "456", "789").withIndex()) {
            val keys = NumericLayout.rows[row]
            assertEquals("第 ${row + 1} 行必须是 3 个键", 3, keys.size)
            assertTrue("第 ${row + 1} 行必须全是直接上屏的字符", keys.all { it.kind == KeyKind.TEXT })
            assertEquals("第 ${row + 1} 行数字顺序不对", expect, keys.joinToString("") { it.code })
        }
    }

    @Test
    fun `五行且各行权重合计一致`() {
        // 行内按键按 weight 分宽度。各行合计不一样的话，同一个 weight 在不同行
        // 的物理宽度会变，换行处按键会看着错位——不是崩溃，但用起来很难受。
        val totals = NumericLayout.rows.map { row -> row.sumOf { it.weight.toDouble() } }
        assertEquals(5, totals.size)
        for ((i, t) in totals.withIndex()) {
            assertEquals("第 ${i + 1} 行权重合计应是 6", 6.0, t, 0.001)
        }
    }

    @Test
    fun `直接上屏的字符里没有字母`() {
        for (c in NumericLayout.textCodes) {
            assertFalse("「$c」是字母，不该出现在数字页", c.any { it in 'a'..'z' || it in 'A'..'Z' })
            assertTrue("「$c」是空串", c.isNotEmpty())
        }
    }

    @Test
    fun `有切回字母页和去符号页的键`() {
        val pages = NumericLayout.rows.flatten().filter { it.kind == KeyKind.PAGE }
        assertTrue("要有回字母页的 ABC", pages.any { it.label == "ABC" && it.code == "letters" })
        assertTrue("要有去符号页的「符」", pages.any { it.label == "符" && it.code == "symbols" })
    }

    @Test
    fun `有空格 回车和删除`() {
        val kinds = NumericLayout.rows.flatten().map { it.kind }.toSet()
        for (need in listOf(KeyKind.SPACE, KeyKind.ENTER, KeyKind.DELETE)) {
            assertTrue("缺功能键 $need", need in kinds)
        }
    }

    @Test
    fun `字符不重复`() {
        val codes = NumericLayout.textCodes
        assertEquals("数字页上有重复字符：$codes", codes.size, codes.toSet().size)
    }
}
