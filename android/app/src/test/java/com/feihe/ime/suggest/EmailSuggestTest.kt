package com.feihe.ime.suggest

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 邮箱后缀联想的规格锁。
 *
 * 这段逻辑的错法都是「偶尔弹出不合适的东西」——在真机上很难描述，只能靠单测把
 * 边界逐条钉死。
 */
class EmailSuggestTest {

    // ---- 该联想的情况 ----

    @Test
    fun `数字后面给出邮箱后缀`() {
        val d = EmailSuggest.digitsFor("12345678")
        assertEquals("12345678", d)
        val domains = EmailSuggest.domainsFor(d)
        assertTrue("必须包含 @qq.com", "@qq.com" in domains)
        assertTrue("必须包含 @163.com", "@163.com" in domains)
    }

    @Test
    fun `中文和空格后面的数字也能联想`() {
        // 末尾必须是数字（光标正停在数字后）。带尾随空格的属于「已经打完」，不联想。
        assertEquals("12345678", EmailSuggest.digitsFor("我的邮箱是 12345678"))
        assertEquals("12345678", EmailSuggest.digitsFor("发到12345678"))
        assertNull("末尾是空格说明这串数字打完了", EmailSuggest.digitsFor("12345678 "))
    }

    @Test
    fun `逗号等标点后的数字也认`() {
        // 标点在**数字前面**不影响；在数字后面说明那段已经结束。
        assertEquals("12345678", EmailSuggest.digitsFor("号码，12345678"))
        assertEquals("12345678", EmailSuggest.digitsFor("12345678"))
        assertNull("末尾标点表示已结束", EmailSuggest.digitsFor("12345678，"))
    }

    @Test
    fun `只有数字没有任何上下文也认`() {
        assertEquals("1234", EmailSuggest.digitsFor("1234"))
    }

    // ---- 不该联想的情况 ----

    @Test
    fun `数字太短不联想`() {
        assertNull("3 位可能只是验证码的一半", EmailSuggest.digitsFor("123"))
        assertNull(EmailSuggest.digitsFor("1"))
        assertNull(EmailSuggest.digitsFor(""))
        assertNull(EmailSuggest.digitsFor(null))
    }

    @Test
    fun `数字太长不联想`() {
        val long = "1".repeat(EmailSuggest.MAX_DIGITS + 1)
        assertNull("16 位不像账号，像订单号", EmailSuggest.digitsFor(long))
    }

    @Test
    fun `已经有 at 号就不联想`() {
        assertNull("已经在打邮箱了，再补整段会重复", EmailSuggest.digitsFor("12345678@"))
        assertNull(EmailSuggest.digitsFor("12345678@qq"))
        assertNull(EmailSuggest.digitsFor("12345678@qq.com"))
    }

    @Test
    fun `字母开头的账号不联想`() {
        assertNull("abc12345678 是用户名，不是邮箱号", EmailSuggest.digitsFor("abc12345678"))
        assertNull(EmailSuggest.digitsFor("user1234"))
    }

    @Test
    fun `版本号和 IP 不联想`() {
        assertNull("1.2.3 是版本号", EmailSuggest.digitsFor("1.2.3"))
        assertNull("192.168.1.1 是 IP", EmailSuggest.digitsFor("192.168.1.1"))
    }

    @Test
    fun `下划线和连字符后的数字不联想`() {
        assertNull(EmailSuggest.digitsFor("order_12345678"))
        assertNull(EmailSuggest.digitsFor("tel-12345678"))
    }

    @Test
    fun `末尾不是数字不联想`() {
        assertNull(EmailSuggest.digitsFor("12345678 "))
        assertNull(EmailSuggest.digitsFor("12345678，"))
    }

    // ---- 候选生成 ----

    @Test
    fun `没有数字就没有候选`() {
        assertTrue(EmailSuggest.domainsFor(null).isEmpty())
        assertTrue(EmailSuggest.domainsFor("").isEmpty())
    }

    @Test
    fun `limit 生效且不会越界`() {
        assertEquals(2, EmailSuggest.domainsFor("12345678", limit = 2).size)
        assertEquals(0, EmailSuggest.domainsFor("12345678", limit = 0).size)
        // limit 超过表长时按表长给
        val all = EmailSuggest.domainsFor("12345678", limit = 999)
        assertEquals(EmailSuggest.DOMAINS.size, all.size)
    }

    @Test
    fun `qq 后缀排第一`() {
        assertEquals("@qq.com", EmailSuggest.domainsFor("12345678").first())
    }

    @Test
    fun `所有后缀都以 at 开头且不重复`() {
        for (d in EmailSuggest.DOMAINS) {
            assertTrue("后缀 $d 必须以 @ 开头", d.startsWith("@"))
        }
        assertEquals("后缀不能重复", EmailSuggest.DOMAINS.size, EmailSuggest.DOMAINS.toSet().size)
    }
}
