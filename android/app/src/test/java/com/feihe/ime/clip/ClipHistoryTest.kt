package com.feihe.ime.clip

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

class ClipHistoryTest {

    private fun at(n: Long) = ClipHistory.Entry("第${n}条", n)

    /** 新内容排最前，旧的往后。 */
    @Test
    fun new_entry_goes_to_head() {
        val old = listOf(at(1), at(2))
        val out = ClipHistory.merge(old, "新内容", 9)
        assertEquals("新内容", out[0].text)
        assertEquals(3, out.size)
    }

    /** 重复内容挪到最前，而不是再存一条。 */
    @Test
    fun duplicate_moves_to_head_instead_of_appending() {
        val old = listOf(at(1), ClipHistory.Entry("同一段", 5), at(3))
        val out = ClipHistory.merge(old, "同一段", 9)
        assertEquals(3, out.size)
        assertEquals("同一段", out[0].text)
        assertEquals(9, out[0].at)
    }

    /** 已经在最前面且内容相同 → 原样返回同一个 List，调用方据此跳过写盘。 */
    @Test
    fun unchanged_list_is_returned_by_identity() {
        val old = listOf(ClipHistory.Entry("同一段", 9), at(1))
        assertSame(old, ClipHistory.merge(old, "同一段", 10))
    }

    /** 空白和超长文本不记。 */
    @Test
    fun blank_and_huge_text_are_ignored() {
        val old = listOf(at(1))
        assertSame(old, ClipHistory.merge(old, "   \n\t ", 2))
        val huge = "x".repeat(ClipHistory.MAX_TEXT + 1)
        assertSame(old, ClipHistory.merge(old, huge, 3))
    }

    /** 超过上限的尾巴丢掉。 */
    @Test
    fun keeps_only_the_limit() {
        var list: List<ClipHistory.Entry> = emptyList()
        repeat(ClipHistory.LIMIT + 5) { i -> list = ClipHistory.merge(list, "第${i}条", i.toLong()) }
        assertEquals(ClipHistory.LIMIT, list.size)
        assertEquals("第${ClipHistory.LIMIT + 4}条", list[0].text)
    }

    /** 换行 / 制表符 / 反斜杠来回一趟一个字都不能变。 */
    @Test
    fun encode_decode_roundtrip_keeps_special_chars() {
        val list = listOf(
            ClipHistory.Entry("第一行\n第二行", 1),
            ClipHistory.Entry("a\tb\\c", 2),
            ClipHistory.Entry("中文，标点。", 3),
        )
        assertEquals(list, ClipHistory.decode(ClipHistory.encode(list)))
    }

    /** 坏行跳过，不抛异常——历史文件被手改坏了也不该让输出法崩。 */
    @Test
    fun decode_skips_broken_lines() {
        val raw = "1\t正常\n坏行没有制表符\nxx\t时间戳不是数字\n2\t另一条"
        val out = ClipHistory.decode(raw)
        assertEquals(2, out.size)
        assertEquals("正常", out[0].text)
        assertTrue(out.all { it.at > 0 })
    }

    @Test
    fun extract_rejects_empty() {
        assertNull(OtpCode.extract(null))
        assertNull(OtpCode.extract("   "))
    }
}
