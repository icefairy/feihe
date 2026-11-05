package com.feihe.ime.suggest

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 联想上下文 + 候选容量的规格锁。
 *
 * 重点钉住两条底线：**密码框一个字都不读**、**末尾是英文不硬凑中文词**。
 * 这两处错了不会崩，但会泄漏隐私或给出莫名其妙的联想，最难在真机上发现。
 */
class SuggestPolicyTest {

    // ── HostContext.lastWord ───────────────────────────────────────

    @Test
    fun `末尾连续汉字取最后两个`() {
        assertEquals("北京", HostContext.lastWord("我今天去了北京"))
        assertEquals("单字就是单字", "时", HostContext.lastWord("时"))
        assertEquals("我去", HostContext.lastWord("a 我去"))
    }

    @Test
    fun `尾部标点和空白先剥掉`() {
        assertEquals("北京", HostContext.lastWord("我今天去了北京，"))
        assertEquals("北京", HostContext.lastWord("我今天去了北京。 "))
        assertEquals("北京", HostContext.lastWord("北京!!!"))
    }

    @Test
    fun `末尾是英文或数字就不猜中文词`() {
        assertEquals("", HostContext.lastWord("发我 abc"))
        assertEquals("", HostContext.lastWord("订单号 12345"))
        assertEquals("", HostContext.lastWord("北京abc"))
    }

    @Test
    fun `没有汉字时空串`() {
        assertEquals("", HostContext.lastWord(""))
        assertEquals("", HostContext.lastWord("   "))
        assertEquals("", HostContext.lastWord("hello world"))
        assertEquals("", HostContext.lastWord("，。！"))
        assertEquals("", HostContext.lastWord(null))
    }

    @Test
    fun `最多取两个汉字不会把整句当词`() {
        // 取光标前最后两个字，而不是从句子开头截两个
        assertEquals("和国", HostContext.lastWord("中华人民共和国"))
        assertEquals("民共", HostContext.lastWord("中华人民共"))
    }

    // ── HostContext.canRead ───────────────────────────────────────

    @Test
    fun `普通文本框可读`() {
        // TYPE_CLASS_TEXT | TYPE_TEXT_VARIATION_NORMAL
        assertTrue(HostContext.canRead(0x00000001))
        // 带自动更正标志的普通文本框
        assertTrue(HostContext.canRead(0x00010001))
    }

    @Test
    fun `密码框一律不可读`() {
        for (v in listOf(0x00000080, 0x00000090, 0x000000e0, 0x00000010)) {
            assertFalse("variation=0x${v.toString(16)} 不该可读", HostContext.canRead(0x00000001 or v))
        }
        // 数字密码
        assertFalse(HostContext.canRead(0x00000002 or 0x00000010))
    }

    @Test
    fun `宿主声明不要建议时不读`() {
        assertFalse(HostContext.canRead(0x00000001 or 0x00080000))
    }

    @Test
    fun `TYPE_NULL 不读`() {
        assertFalse(HostContext.canRead(0))
    }

    // ── CandidateBarPolicy ────────────────────────────────────────

    @Test
    fun `没有联想时按用户上限给满`() {
        assertEquals(9, CandidateBarPolicy.candidateSlots(9, false))
        assertEquals(7, CandidateBarPolicy.candidateSlots(7, false))
        assertEquals(5, CandidateBarPolicy.candidateSlots(5, false))
    }

    @Test
    fun `有联想时候选压到 6 个`() {
        assertEquals(6, CandidateBarPolicy.candidateSlots(9, true))
        assertEquals(6, CandidateBarPolicy.candidateSlots(7, true))
        assertEquals("本来就不满 6 的不动", 5, CandidateBarPolicy.candidateSlots(5, true))
    }

    @Test
    fun `非法上限回落到默认`() {
        assertEquals(9, CandidateBarPolicy.sanitize(0))
        assertEquals(9, CandidateBarPolicy.sanitize(100))
        assertEquals(9, CandidateBarPolicy.sanitize(-1))
        assertEquals(7, CandidateBarPolicy.sanitize(7))
    }

    @Test
    fun `档位只有 5 7 9`() {
        assertEquals(listOf(5, 7, 9), CandidateBarPolicy.LIMITS)
        assertEquals(CandidateBarPolicy.DEFAULT_LIMIT, CandidateBarPolicy.LIMITS.last())
    }
}
