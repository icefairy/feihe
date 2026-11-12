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
    fun `数字页不出现全角标点 小数点必须是半角`() {
        // 用户明确要求：数字键盘不该有中文句号，应该换成英文并能当小数点。
        // 这不是偏好问题——`12。5` 在银行/表单/Excel 里都不是 12.5，
        // 而中文键盘原本连半角 `.` 都打不出来，等于小额支付这条路是断的。
        val codes = NumericLayout.textCodes.toSet()
        assertFalse("数字页不该出现全角句号「。」", "。" in codes)
        assertFalse("数字页不该出现全角逗号「，」", "，" in codes)
        assertTrue("数字页必须有半角小数点「.」", "." in codes)
        assertTrue("数字页必须有半角逗号「,」", "," in codes)
    }

    @Test
    fun `小数点就在底排 和 0 相邻`() {
        // 输 12.5 时手不用换位置。放在别处（比如符号页）等于没修。
        val bottom = NumericLayout.rows[3]
        assertEquals("底排应是 , 0 . 三键", listOf(",", "0", "."), bottom.map { it.code })
    }

    @Test
    fun `中英模式下数字页完全一致`() {
        // 数字页的用途是中英共通的（算账、填表、输版本号），
        // 分两套只会让「中文下打不出小数点」这种坑重新长回来。
        val zh = NumericLayout.rowsOf(true).flatten().map { it.code }
        val en = NumericLayout.rowsOf(false).flatten().map { it.code }
        assertEquals("中英数字页不该有差异", zh, en)
    }

    @Test
    fun `小数点直接上屏 不进组合串`() {
        // 必须是 TEXT 而不是 PUNCT：PUNCT 会先把候选上屏再粘标点，
        // 而在数字页上用户往往什么都没在打组合串，走了 PUNCT 只会多一次无用操作。
        val dot = NumericLayout.rows.flatten().first { it.code == "." }
        assertEquals(KeyKind.TEXT, dot.kind)
    }

    // ── 上滑运算符号（2026-11-12 新增）────────────────────────────

    @Test
    fun `数字键都有上滑字符`() {
        for (d in '0'..'9') {
            val key = NumericLayout.rows.flatten().first { it.code == d.toString() }
            assertTrue("数字 $d 没有上滑字符", key.swipe != null)
        }
    }

    @Test
    fun `上滑有加减乘除和等号`() {
        val swipes = NumericLayout.swipeCodes
        for (op in listOf("+", "-", "×", "÷", "=", "%")) {
            assertTrue("数字页上滑缺运算符 $op", op in swipes)
        }
    }

    @Test
    fun `上滑里也有斜杠`() {
        assertTrue("数字页上滑也该有 /，填日期/分数时常用", "/" in NumericLayout.swipeCodes)
    }

    @Test
    fun `上滑字符和主字符不撞车`() {
        val mains = NumericLayout.textCodes.toSet()
        for (s in NumericLayout.swipeCodes) {
            assertFalse("上滑的「$s」和主字符重复了", s in mains)
        }
    }

    @Test
    fun `上滑字符互不重复`() {
        val swipes = NumericLayout.swipeCodes
        assertEquals("数字页上滑有重复：$swipes", swipes.size, swipes.toSet().size)
    }
}
