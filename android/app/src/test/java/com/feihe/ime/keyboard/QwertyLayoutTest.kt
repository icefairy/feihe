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
}
