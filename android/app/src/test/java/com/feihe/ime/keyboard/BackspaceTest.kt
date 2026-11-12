package com.feihe.ime.keyboard

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * 退格删几个 code unit 的规格锁。
 *
 * 用户报的是「emoji 退格只能删一半」：固定 `deleteSurroundingText(1, 0)` 会把
 * 代理对切成两半，留下一个孤立代理项显示成乱码。
 */
class BackspaceTest {

    @Test
    fun `普通汉字删一个`() {
        assertEquals(1, Backspace.unitsToDelete("字"))
        assertEquals(1, Backspace.unitsToDelete("hello"))
        assertEquals(1, Backspace.unitsToDelete("123"))
    }

    @Test
    fun `emoji 删两个`() {
        // 😊 U+1F60A 是代理对，占两个 code unit
        val s = "😊"
        assertEquals("emoji 字符串长度必须为 2 才说明是代理对", 2, s.length)
        assertEquals(2, Backspace.unitsToDelete(s))
    }

    @Test
    fun `汉字后面跟 emoji 也只删 emoji`() {
        assertEquals(2, Backspace.unitsToDelete("字😊"))
        assertEquals(2, Backspace.unitsToDelete("a😊"))
    }

    @Test
    fun `多个 emoji 只删最后一个`() {
        assertEquals(2, Backspace.unitsToDelete("😊😊"))
    }

    @Test
    fun `空和 null 退回删一个`() {
        // 保守：读不到就删 1，宁可用户再按一次，也不能一次删两个真字
        assertEquals(1, Backspace.unitsToDelete(""))
        assertEquals(1, Backspace.unitsToDelete(null))
    }

    @Test
    fun `孤立代理项也只删一个`() {
        // 错误数据（半个代理对）不该让我们删 2 把前面的真字一起吃掉
        val lonely = "\uD83D" // 只有高位代理，没有低位
        assertEquals(1, Backspace.unitsToDelete(lonely))
    }

    @Test
    fun `重载版本行为一致`() {
        val s = "字😊"
        assertEquals(
            Backspace.unitsToDelete(s),
            Backspace.unitsToDelete(s[s.length - 1], s[s.length - 2]),
        )
        assertEquals(1, Backspace.unitsToDelete('字', 'a'))
        // 只有低位代理、没有前一个字符：算不出代理对，保守删 1
        assertEquals(1, Backspace.unitsToDelete("😊"[1], null))
    }
}
