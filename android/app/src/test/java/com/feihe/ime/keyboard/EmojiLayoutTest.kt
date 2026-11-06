package com.feihe.ime.keyboard

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 表情页的规格锁。
 *
 * 这一页**没有引擎兜底**：打错了没有候选可以纠，页面上摆什么就只能打什么。
 * 所以这里锁三件事：不许重复（同一个表情占两个格是浪费）、必须能回到其它页
 * （回不去就是死路）、高度必须和数字/符号页一致（换页时键盘不能长高变矮）。
 */
class EmojiLayoutTest {

    @Test
    fun `表情不重复且非空`() {
        val codes = EmojiLayout.textCodes
        for (c in codes) assertTrue("表情是空串", c.isNotEmpty())
        assertEquals("表情页上有重复：$codes", codes.size, codes.toSet().size)
    }

    @Test
    fun `表情不会混进字母或数字`() {
        for (c in EmojiLayout.textCodes) {
            assertTrue(
                "「$c」是 ASCII 字符，不该出现在表情页",
                c.none { it in 'a'..'z' || it in 'A'..'Z' || it in '0'..'9' },
            )
        }
    }

    @Test
    fun `三个方向都能切走并保留空格回车删除`() {
        val pages = EmojiLayout.rows.flatten().filter { it.kind == KeyKind.PAGE }
        assertTrue("要有回字母页的 ABC", pages.any { it.label == "ABC" && it.code == "letters" })
        assertTrue("要有回数字页的 123", pages.any { it.label == "123" && it.code == "digits" })
        assertTrue("要有去符号页的「符」", pages.any { it.label == "符" && it.code == "symbols" })

        val kinds = EmojiLayout.rows.flatten().map { it.kind }.toSet()
        for (need in listOf(KeyKind.SPACE, KeyKind.ENTER, KeyKind.DELETE)) {
            assertTrue("缺功能键 $need", need in kinds)
        }
    }

    @Test
    fun `三个紧凑页的行数与行权重合计一致`() {
        // 高度只由「行数 × 行高」决定，所以切页不跳的前提是**行数相同**；
        // 而「各行键宽看着一样」的前提是**每行权重合计相同**。
        // 列数可以不同（表情页 7 列、数字页 3 列），那不是约束。
        for (layout in listOf(NumericLayout.rows, SymbolLayout.rows, EmojiLayout.rows)) {
            assertEquals("紧凑页必须是 5 行", 5, layout.size)
            for ((i, row) in layout.withIndex()) {
                val total = row.sumOf { it.weight.toDouble() }
                assertEquals("第 ${i + 1} 行权重合计应是 6", 6.0, total, 0.001)
            }
        }
    }

    @Test
    fun `从数量页和符号页都能直接到表情页`() {
        // 入口只出现在字母页以外是不够的：用户在符号页想加个表情，
        // 不该先绕回字母页。
        for (layout in listOf(NumericLayout.rows, SymbolLayout.rows)) {
            val hit = layout.flatten().any { it.kind == KeyKind.PAGE && it.code == "emoji" }
            assertTrue("这一页没有去表情页的入口", hit)
        }
    }
}
