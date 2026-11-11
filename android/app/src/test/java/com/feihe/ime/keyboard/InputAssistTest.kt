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

    // ── 双击锁定（2026-11-12 新行为）──────────────────────────────
    //
    // 这一组钉的是「按两下锁、再按两下解」：它是用户明确要的手势，而这个手势
    // 唯一的风险是把自己和「连按两下打两个大写」搞混。判定窗口 = 300ms。

    @Test
    fun `双击上档进入锁定`() {
        val t0 = 1000L
        // 第一下：OFF → ONCE
        val (s1, d1) = ShiftMachine.onTapWithTime(ShiftState.OFF, 0L, t0)
        assertEquals(ShiftState.ONCE, s1)
        assertFalse("第一下不该判成双击", d1)
        // 第二下（150ms 后）：ONCE → LOCKED
        val (s2, d2) = ShiftMachine.onTapWithTime(s1, t0, t0 + 150)
        assertEquals(ShiftState.LOCKED, s2)
        assertTrue("第二下应判成双击", d2)
    }

    @Test
    fun `锁定态双击解锁`() {
        val t = 5000L
        val (s, d) = ShiftMachine.onTapWithTime(ShiftState.LOCKED, t, t + 100)
        assertEquals(ShiftState.OFF, s)
        assertTrue(d)
    }

    @Test
    fun `超过双击窗口就当成两次独立单击`() {
        val t = 1000L
        val gap = ShiftMachine.DOUBLE_TAP_MS + 1
        // 第一下 ONCE，第二下（超时）应落回 OFF，而不是锁定
        val (s, d) = ShiftMachine.onTapWithTime(ShiftState.ONCE, t, t + gap)
        assertEquals(ShiftState.OFF, s)
        assertFalse("超窗口不该判成双击", d)
    }

    @Test
    fun `刚好等于窗口边界也算双击`() {
        val t = 1000L
        val (s, d) = ShiftMachine.onTapWithTime(ShiftState.ONCE, t, t + ShiftMachine.DOUBLE_TAP_MS)
        assertEquals(ShiftState.LOCKED, s)
        assertTrue(d)
    }

    @Test
    fun `没有上一次点击时永远不是双击`() {
        val (s, d) = ShiftMachine.onTapWithTime(ShiftState.OFF, 0L, 999999L)
        assertEquals(ShiftState.ONCE, s)
        assertFalse(d)
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
