package com.feihe.ime.keyboard

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 数字/符号页的规格锁。
 *
 * 这一页最怕的不是「少一个符号」，而是**缺数字**——字母页上没有任何地方能打
 * `0`-`9`，这一页是唯一入口，少一个就是永久打不出来。
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
    fun `各行权重合计一致`() {
        // 行内按键按 weight 分宽度。四行合计不一样的话，换行处按键宽度会跳，
        // 手指按上去的落点和看到的位置对不上——这种错不是崩溃，但用起来很难受。
        val totals = NumericLayout.rows.map { row -> row.sumOf { it.weight.toDouble() } }
        assertEquals(4, totals.size)
        for (t in totals) {
            assertEquals("各行 width 合计要一样，否则键宽会跳", 5.0, t, 0.001)
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
    fun `有切回字母页的键`() {
        val symbol = NumericLayout.rows.flatten().firstOrNull { it.kind == KeyKind.SYMBOL }
        assertTrue("数字页必须有切回去的键", symbol != null)
        assertEquals("ABC", symbol!!.label)
    }

    @Test
    fun `有空格和删除`() {
        val kinds = NumericLayout.rows.flatten().map { it.kind }.toSet()
        assertTrue("缺空格", KeyKind.SPACE in kinds)
        assertTrue("缺删除", KeyKind.DELETE in kinds)
    }

    @Test
    fun `字符不重复`() {
        val codes = NumericLayout.textCodes
        assertEquals("数字页上有重复字符：$codes", codes.size, codes.toSet().size)
    }
}
