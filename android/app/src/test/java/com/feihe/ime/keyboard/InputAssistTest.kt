package com.feihe.ime.keyboard

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 上档状态机 + 数字选词的规格锁。
 *
 * 这两块都是纯逻辑，但错在边界上：一次性大写"打完要不要落下"、点 1 到底选第几个。
 * 放进 InputMethodService 就只能在真机上靠手点，而真机上表现为"偶尔不对"，最难查。
 */
class InputAssistTest {

    // ── 上档 ────────────────────────────────────────────────────────

    @Test
    fun `单击 OFF 进一次性，再点取消`() {
        assertEquals(ShiftState.ONCE, ShiftMachine.onTap(ShiftState.OFF))
        assertEquals(ShiftState.OFF, ShiftMachine.onTap(ShiftState.ONCE))
    }

    @Test
    fun `锁定态单击是解锁而不是再锁一层`() {
        assertEquals(ShiftState.OFF, ShiftMachine.onTap(ShiftState.LOCKED))
    }

    @Test
    fun `长按从任意非锁定态进入锁定`() {
        assertEquals(ShiftState.LOCKED, ShiftMachine.onLongPress(ShiftState.OFF))
        assertEquals(ShiftState.LOCKED, ShiftMachine.onLongPress(ShiftState.ONCE))
    }

    @Test
    fun `长按锁定态是解锁`() {
        assertEquals(ShiftState.OFF, ShiftMachine.onLongPress(ShiftState.LOCKED))
    }

    @Test
    fun `一次性大写打完一个字母就落下，锁定态不落`() {
        assertEquals(ShiftState.OFF, ShiftMachine.afterChar(ShiftState.ONCE))
        assertEquals(ShiftState.LOCKED, ShiftMachine.afterChar(ShiftState.LOCKED))
        assertEquals(ShiftState.OFF, ShiftMachine.afterChar(ShiftState.OFF))
    }

    @Test
    fun `锁定态连打三个字母全大写`() {
        var s = ShiftState.LOCKED
        val out = (1..3).map {
            val upper = ShiftMachine.upper(s)
            s = ShiftMachine.afterChar(s)
            upper
        }
        assertEquals(listOf(true, true, true), out)
        assertEquals(ShiftState.LOCKED, s)
    }

    @Test
    fun `一次性大写只影响第一个字母`() {
        var s = ShiftMachine.onTap(ShiftState.OFF)
        val out = (1..3).map {
            val upper = ShiftMachine.upper(s)
            s = ShiftMachine.afterChar(s)
            upper
        }
        assertEquals(listOf(true, false, false), out)
    }

    @Test
    fun `锁定态下 upper 恒真`() {
        assertTrue(ShiftMachine.upper(ShiftState.ONCE))
        assertTrue(ShiftMachine.upper(ShiftState.LOCKED))
        assertFalse(ShiftMachine.upper(ShiftState.OFF))
    }

    // ── 数字选词 ────────────────────────────────────────────────────

    @Test
    fun `没候选时数字行是空的，不常驻`() {
        assertTrue(NumberSelect.digits(0).isEmpty())
        assertTrue(NumberSelect.digits(-1).isEmpty())
    }

    @Test
    fun `候选少于 9 个时数字键数量等于候选数`() {
        assertEquals(listOf(1, 2, 3), NumberSelect.digits(3).toList())
    }

    @Test
    fun `候选超过 9 个也只给 9 个数字键`() {
        assertEquals(9, NumberSelect.digits(20).count())
        assertEquals(9, NumberSelect.digits(20).last)
    }

    @Test
    fun `点 1 选第 0 个候选点 9 选第 8 个`() {
        assertEquals(0, NumberSelect.indexOf(1, 9))
        assertEquals(8, NumberSelect.indexOf(9, 9))
    }

    @Test
    fun `数字超出候选数或范围时忽略`() {
        assertNull(NumberSelect.indexOf(4, 3))
        assertNull(NumberSelect.indexOf(0, 9))
        assertNull(NumberSelect.indexOf(10, 20))
        assertNull(NumberSelect.indexOf(-1, 9))
    }
}
