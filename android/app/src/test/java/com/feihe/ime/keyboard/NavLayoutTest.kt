package com.feihe.ime.keyboard

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 光标/编辑控制页的规格锁。
 *
 * 方向键最怕的是**移动方向反了**或者**越界把光标甩到 0**——这两种错在真机上表现
 * 都是「按了没反应」或「光标乱跳」，而且只有长文本里才看得出来。所以这里把
 * 「边界」「跨行」「列对齐」三种情况全部钉死。
 */
class NavLayoutTest {

    @Test
    fun `控制页有五排且每排权重合计是 6`() {
        val totals = NavLayout.rows.map { row -> row.sumOf { it.weight.toDouble() } }
        assertEquals(5, totals.size)
        for ((i, t) in totals.withIndex()) {
            assertEquals("第 ${i + 1} 排权重合计应是 6", 6.0, t, 0.001)
        }
    }

    @Test
    fun `四个方向键都在`() {
        val actions = NavLayout.rows.flatten()
            .filter { it.kind == KeyKind.NAV }
            .map { it.code }
        for (need in listOf("LEFT", "RIGHT", "UP", "DOWN")) {
            assertTrue("缺方向键 $need", need in actions)
        }
    }

    @Test
    fun `复制粘贴剪切全选都在`() {
        val actions = NavLayout.rows.flatten()
            .filter { it.kind == KeyKind.NAV }
            .map { it.code }
        for (need in listOf("COPY", "PASTE", "CUT", "SELECT_ALL")) {
            assertTrue("缺 $need", need in actions)
        }
    }

    /**
     * 方向键的 code 必须能被 [NavAction.of] 认出来。
     *
     * 这条是防「改了枚举名忘了改布局表」——那种错编译能过，按下去毫无反应。
     */
    @Test
    fun `每个 NAV 键的 code 都能解析成动作`() {
        for (k in NavLayout.rows.flatten().filter { it.kind == KeyKind.NAV }) {
            assertTrue("键「${k.label}」的 code=${k.code} 无法解析", NavAction.of(k.code) != null)
        }
    }

    @Test
    fun `控制页有切回字母页和数字页的键`() {
        val pages = NavLayout.rows.flatten().filter { it.kind == KeyKind.PAGE }
        assertTrue("缺回字母页", pages.any { it.code == "letters" })
        assertTrue("缺去数字页", pages.any { it.code == "digits" })
    }

    // ── NavCursor 的边界 ──────────────────────────────────────────

    @Test
    fun `左移到头停在 0，不会变成负数`() {
        assertEquals(0, NavCursor.move("abc", 0, NavAction.LEFT))
        assertEquals(0, NavCursor.move("abc", 1, NavAction.LEFT))
    }

    @Test
    fun `右移到末尾停住，不会越界`() {
        assertEquals(3, NavCursor.move("abc", 3, NavAction.RIGHT))
        assertEquals(3, NavCursor.move("abc", 2, NavAction.RIGHT))
    }

    @Test
    fun `上移到第一行就没有目标`() {
        assertNull(NavCursor.move("abc\ndef", 1, NavAction.UP))
    }

    @Test
    fun `下移到最后一行就没有目标`() {
        assertNull(NavCursor.move("abc\ndef", 5, NavAction.DOWN))
    }

    @Test
    fun `上下移保持列号`() {
        // "abc\ndef"：第二行起始 4，列 1 的 d 后面是 e
        assertEquals(1, NavCursor.move("abc\ndef", 5, NavAction.UP))  // 第二行第2个 → 第一行第2个
        assertEquals(5, NavCursor.move("abc\ndef", 1, NavAction.DOWN)) // 第一行第2个 → 第二行第2个
    }

    @Test
    fun `目标行比当前列短时停在目标行行尾`() {
        // "abcdefg\nhi"：'a'..'g' 是 0..6，换行符在 7，'h' 在 8，'i' 在 9，长度 10
        // 第一行第 6 列往下移 → 第二行只有 2 个字符 → 停在行尾（'i' 之后 = 10），
        // 不该跨到文本末尾以外，也不该留在 8+6=14 那种越界位置。
        assertEquals(10, NavCursor.move("abcdefg\nhi", 6, NavAction.DOWN))
    }

    @Test
    fun `行首行尾`() {
        // "abc\ndefg\nhi"：'a'0 'b'1 'c'2 '\n'3 'd'4 'e'5 'f'6 'g'7 '\n'8 'h'9 'i'10
        val t = "abc\ndefg\nhi"
        assertEquals("第一行行首", 0, NavCursor.lineStart(t, 2))
        assertEquals("第二行行首", 4, NavCursor.lineStart(t, 6))
        assertEquals("第一行行尾（换行符位置）", 3, NavCursor.lineEnd(t, 0))
        assertEquals("第二行行尾（换行符位置）", 8, NavCursor.lineEnd(t, 5))
        assertEquals("最后一行行尾是文本末尾", t.length, NavCursor.lineEnd(t, 10))
    }

    @Test
    fun `单行文本的行首行尾就是文本两端`() {
        assertEquals(0, NavCursor.lineStart("hello", 3))
        assertEquals(5, NavCursor.lineEnd("hello", 3))
    }

    @Test
    fun `行号计算`() {
        val t = "a\nb\nc"
        assertEquals(0, NavCursor.lineOf(t, 0))
        assertEquals(1, NavCursor.lineOf(t, 2))
        assertEquals(2, NavCursor.lineOf(t, 4))
    }

    @Test
    fun `非移动类动作返回 null`() {
        for (a in listOf(NavAction.COPY, NavAction.PASTE, NavAction.SELECT_ALL, NavAction.CUT)) {
            assertNull("$a 不是移动动作，不该返回位置", NavCursor.move("abc", 1, a))
        }
    }

    @Test
    fun `越界的位置会被夹回来`() {
        assertEquals(3, NavCursor.move("abc", 999, NavAction.RIGHT))
        assertEquals(0, NavCursor.move("abc", -5, NavAction.LEFT))
    }

    @Test
    fun `NavAction of 认不出时返回 null`() {
        assertNull(NavAction.of("NOT_A_REAL_ACTION"))
        assertEquals(NavAction.COPY, NavAction.of("COPY"))
    }

    @Test
    fun `空文本上的移动不会崩`() {
        assertEquals(0, NavCursor.move("", 0, NavAction.LEFT))
        assertEquals(0, NavCursor.move("", 0, NavAction.RIGHT))
        assertNull(NavCursor.move("", 0, NavAction.UP))
    }

    /** 空白占位键要真的不可见：label 和 code 都是空串。 */
    @Test
    fun `占位键是空 label 空 code`() {
        val blanks = NavLayout.rows.flatten().filter { it.kind == KeyKind.TEXT && it.label.isEmpty() }
        assertTrue("十字布局需要占位键撑住中间那格", blanks.isNotEmpty())
        for (b in blanks) {
            assertEquals("", b.code)
            assertFalse("占位键不该有上滑", b.swipe != null)
        }
    }
}
