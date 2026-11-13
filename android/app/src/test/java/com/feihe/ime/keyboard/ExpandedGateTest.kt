package com.feihe.ime.keyboard

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 上屏 / 清空之后，展开态必须自动收回。
 *
 * ## 钉的是哪个 bug
 *
 * 用户原话：「自组词展开选字儿之后，选完最后一个字儿，选区空了但还留了个空白
 * 面板大概有一行左右的高度，底下还有一个『收起』的点击区域，需要选完之后
 * 还需要用户手动点一次。这个应该去掉，做成自动的——最后一个字选完就自动
 * 上屏，然后候选区域收起来变回正常的键盘。」
 *
 * 根因：上屏清的是**数据**（candidates / segmentState），而 `candidateExpanded`
 * 是**呈现标志**，没人动。展开态是把键盘藏起来、借高度铺候选的，于是继续撑着 →
 * 一块空白 + 一根多余的收起条 + 键盘回不来。
 *
 * ## 为什么这组用例不自己复刻判据
 *
 * 第一版测试在测试类里重写了一遍 `shouldKeepExpanded`，结果把生产代码的修法
 * 撤掉后测试**照样通过**——等于什么都没钉住。所以判据抽进了 [ExpandedGate]
 * 这个纯函数，测试直接调它：生产代码改坏，测试必红。
 */
class ExpandedGateTest {

    @Test
    fun `上屏后候选清空 —— 必须收回`() {
        assertFalse(
            "候选已清空还想保持展开，会留下一块空白面板 + 多余的收起条",
            ExpandedGate.keepsExpanded(windowSize = 0, segmentFocus = -1, segmentCount = 0),
        )
    }

    @Test
    fun `还有候选可画 —— 保持展开`() {
        assertTrue(ExpandedGate.keepsExpanded(windowSize = 5, segmentFocus = -1, segmentCount = 0))
        assertTrue(ExpandedGate.keepsExpanded(windowSize = 1, segmentFocus = -1, segmentCount = 0))
    }

    @Test
    fun `正停在某一段 —— 即使候选窗口为空也要保持`() {
        // 形码把候选筛空时 window 会是 0，但操作行（‹ 返回 / 形）还在，
        // 那个面板是用户唯一的退路，不能收
        assertTrue(
            ExpandedGate.keepsExpanded(windowSize = 0, segmentFocus = 0, segmentCount = 3),
        )
        assertTrue(
            ExpandedGate.keepsExpanded(windowSize = 0, segmentFocus = 2, segmentCount = 3),
        )
    }

    @Test
    fun `选完最后一段 —— focus 归 -1 后就该收`() {
        // 用户报的具体场景：三段词，最后一段选完 → focus 变 -1
        assertFalse(
            ExpandedGate.keepsExpanded(windowSize = 0, segmentFocus = -1, segmentCount = 3),
        )
    }

    @Test
    fun `focus 越界当作没展开 —— 不能因为脏下标把键盘藏住`() {
        assertFalse(ExpandedGate.keepsExpanded(windowSize = 0, segmentFocus = 5, segmentCount = 3))
        assertFalse(ExpandedGate.keepsExpanded(windowSize = 0, segmentFocus = 0, segmentCount = 0))
    }

    @Test
    fun `普通拼音展开后点选候选 —— 同一个洞`() {
        // 不是分段态专属：普通候选展开 → 点一个词 → 候选清空 → 同样要收
        assertFalse(ExpandedGate.keepsExpanded(windowSize = 0, segmentFocus = -1, segmentCount = 0))
    }
}
