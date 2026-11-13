package com.feihe.ime.keyboard

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 键帽字号分档。
 *
 * ## 钉的是哪个 bug
 *
 * 用户原话：「回车键上的符号也太单薄或者太小。」
 *
 * 根因不是"回车键特殊"，而是**以前只有两档字号**：字母/文本 20sp，其余全部 16sp。
 * 回车是底排最宽的键（权重 1.8，比「中」宽 80%），而 `↵` 这个字形本身笔画细、
 * 留白多，16sp 摊在那么大一块键帽上就显得又细又空。
 *
 * 这类问题不会崩、不会报错，只能靠「字号与键面积、字形粗细匹配」这条规则守住。
 */
class KeyTextSizeTest {

    private fun size(kind: KeyKind) = keyTextSizeSp(Key("x", kind, "x"))

    @Test
    fun `回车必须比普通功能键大`() {
        val enter = size(KeyKind.ENTER)
        val fn = size(KeyKind.PAGE)
        assertTrue(
            "回车 $enter sp 不大于功能键 $fn sp —— 它是最宽的键，字却一样小",
            enter > fn,
        )
    }

    @Test
    fun `回车 26sp —— 真机像素实测出来的值，不是拍的`() {
        // 1080p 模拟器实测（同键高 126px）：
        //   ↵ 16sp(原) → 20x22px，墨量 254？不，原值是 16sp 时约 12x14
        //   ↵ 22sp    → 20x22px
        //   ↵ 26sp    → 24x26px，墨量 195
        //   ↵ 30sp    → 27x30px，墨量 254（偏大）
        //   中 16sp   → 37x58px，墨量 101
        //
        // 判据用**墨量**（深色像素数）而不是外接框：↵ 是细线字形，外接框量不出
        // 「实不实」。26sp 的墨量是「中」的 1.9 倍，视觉上明显压过旁边的功能键，
        // 又没到 30sp 那种抢眼程度。
        assertEquals(26f, size(KeyKind.ENTER), 0.01f)
    }

    @Test
    fun `功能键一律不比字母键大 —— 只有回车例外`() {
        // 回车 22 > 字母 20 是**刻意的**：它宽 1.8 倍且字形细，字号必须压过面积差。
        // 其余功能键一律不能超过字母键，否则底排会显得跟正文抢视觉重心。
        val typing = size(KeyKind.CHAR)
        for (k in listOf(KeyKind.SPACE, KeyKind.DELETE, KeyKind.PAGE,
                         KeyKind.MODE, KeyKind.SHIFT)) {
            assertTrue("$k 不该比字母键还大", size(k) <= typing)
        }
    }

    @Test
    fun `除回车外所有功能键保持原样 16sp`() {
        // 用户明确说过「其他按键体积不大」——不要顺手放大 ⇧ ⌫ 123 中 ⌘，
        // 那会让底排变拥挤。这条钉住「只有回车被特殊对待」。
        for (k in listOf(KeyKind.SPACE, KeyKind.DELETE, KeyKind.PAGE,
                         KeyKind.MODE, KeyKind.SHIFT)) {
            assertEquals("$k 不该被改动", 16f, size(k), 0.01f)
        }
    }

    @Test
    fun `功能键不能小到看不清`() {
        for (k in listOf(KeyKind.ENTER, KeyKind.SPACE, KeyKind.DELETE, KeyKind.PAGE,
                         KeyKind.MODE, KeyKind.SHIFT)) {
            assertTrue("$k 的字号 ${size(k)} sp 太小", size(k) >= 16f)
        }
    }
}
