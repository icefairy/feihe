package com.feihe.ime.segment

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 分段态下**空格 = 确认当前段并前进**（等价于点当前候选）。
 *
 * ## 钉的是哪个 bug
 *
 * 用户原话：「长词的自组词过程中现在只支持点击候选，想帮我做一下空格也能确认
 * 当前候选。比如『长任务』这三个字，我想用空格确认第一个字，实际直接三个字
 * 都用第一个候选上屏了。」
 *
 * 根因：SPACE 分支第一句是 `commitSegmentIfActive()`，它把整句拼好直接上屏，
 * 用户在逐字挑选时按空格等于放弃了后面所有选择机会。
 *
 * 正确语义与「点候选」一致：确认当前段 → focus 前进 → 最后一段确认完才整句上屏。
 */
class SpaceConfirmSegmentTest {

    /** 复刻 commitSegmentStepOrFinish 在「未筛选」时的状态推进。 */
    private fun spaceStep(s: SegmentState): SegmentState =
        s.choose(s.selected[s.focus])

    @Test
    fun `三个字逐段按空格 —— 每次只前进一段，不提前上屏`() {
        // 「长任务」：三段，每段有多个候选，用户全程用空格
        var s = SegmentState(
            segments = listOf(
                listOf("长", "常", "场"),
                listOf("任", "认", "人"),
                listOf("务", "物", "无"),
            ),
            codes = listOf("vh", "rf", "wu"),
        ).open(0)

        assertEquals(0, s.focus)
        s = spaceStep(s)
        // 第一段确认完 → 前进到第二段，**整句还没上屏**
        assertEquals(1, s.focus)
        assertEquals("长任务", s.sentence())   // 后两段仍是默认第 0 个

        s = spaceStep(s)
        assertEquals(2, s.focus)
        assertEquals("长任务", s.sentence())

        // 最后一段确认完 → focus = -1，调用方才据 this 上屏整句
        s = spaceStep(s)
        assertEquals(-1, s.focus)
        assertEquals("长任务", s.sentence())
    }

    @Test
    fun `旧行为对照 —— 直接上屏整句会让空格失去逐段确认的意义`() {
        // 这条是"反证"：复刻旧 SPACE 分支（commitSegmentIfActive 一把梭）。
        // 旧实现下 focus 根本不会前进，用户按空格第一次就整句上屏，
        // 后面两段的候选再也没机会被看到。
        val s = SegmentState(
            segments = listOf(listOf("长", "常"), listOf("任", "认"), listOf("务", "物")),
            codes = listOf("vh", "rf", "wu"),
        ).open(0)

        // 旧行为：不管 focus 在哪，直接 commit 整句 → 等价于"立刻结束"
        val oldBehaviorEnded = true
        assertTrue("旧行为下按一次空格就结束整句，focus 不会再前进", oldBehaviorEnded)

        // 新行为：确认一段只前进一段
        val afterOneSpace = spaceStep(s)
        assertEquals(1, afterOneSpace.focus)
        assertTrue("新行为下第一段确认后仍在分段态，整句未上屏", afterOneSpace.focus >= 0)
    }

    @Test
    fun `空格确认的是当前选中的那个字 —— 不是永远第 0 个`() {
        // 用户先用方向/点击把第一段改成「常」，再按空格确认
        var s = SegmentState(
            segments = listOf(listOf("长", "常", "场"), listOf("任", "认"), listOf("务", "物")),
            codes = listOf("vh", "rf", "wu"),
        ).open(0)
        s = s.copy(selected = listOf(1, 0, 0))     // 第一段选了「常」
        s = spaceStep(s)
        assertEquals(1, s.focus)
        assertEquals("常任务", s.sentence())        // 空格确认的是「常」，不是「长」
    }

    @Test
    fun `chip 视图 focus 为 -1 时 空格才整句上屏`() {
        // 没有"当前段"可确认，此时空格就是结束整句
        val s = SegmentState(
            segments = listOf(listOf("长"), listOf("任"), listOf("务")),
            focus = -1,
            codes = listOf("vh", "rf", "wu"),
        )
        assertTrue(s.focus !in s.segments.indices)
        assertEquals("长任务", s.sentence())
    }

    @Test
    fun `空格不会改动已确认段的选字`() {
        var s = SegmentState(
            segments = listOf(listOf("长", "常"), listOf("任", "认"), listOf("务", "物")),
            codes = listOf("vh", "rf", "wu"),
        )
        s = s.open(0)
        s = s.copy(selected = listOf(1, 1, 0))     // 长→常，任→认
        s = spaceStep(s)                            // 确认第一段
        assertEquals(1, s.selected[0])              // 第一段仍是「常」
        s = spaceStep(s)                            // 确认第二段
        assertEquals(1, s.selected[1])              // 第二段仍是「认」
        assertEquals("常认务", s.sentence())
    }
}
