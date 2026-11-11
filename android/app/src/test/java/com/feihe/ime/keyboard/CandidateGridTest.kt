package com.feihe.ime.keyboard

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 候选条展开（多行）的规格锁。
 *
 * 这段逻辑错了都不会崩，只会「少显示几个候选」或者「翻页跳过了一批」——真机上
 * 根本注意不到，但用户会觉得候选条不听话。所以把行数、页大小、越界夹取全部钉住。
 */
class CandidateGridTest {

    @Test
    fun `不足一屏时按实际个数换行`() {
        assertEquals(0, CandidateGrid.rows(0))
        assertEquals(1, CandidateGrid.rows(1))
        assertEquals(1, CandidateGrid.rows(6))
        assertEquals(2, CandidateGrid.rows(7))
        assertEquals(3, CandidateGrid.rows(18))
    }

    @Test
    fun `最多显示三行`() {
        assertEquals(3, CandidateGrid.rows(45))
        assertEquals(3 * CandidateGrid.PER_ROW, CandidateGrid.expandedCount(45))
    }

    @Test
    fun `收起态一页就是设置里的候选个数`() {
        assertEquals(5, CandidateGrid.pageSize(45, expanded = false, collapseSize = 5))
        assertEquals(9, CandidateGrid.pageSize(45, expanded = false, collapseSize = 9))
    }

    @Test
    fun `展开态一页是行乘列`() {
        assertEquals(18, CandidateGrid.pageSize(45, expanded = true, collapseSize = 9))
        // 只有 4 个候选时，一页就是 4 个——否则 hasNext 会误报「还有下一页」
        assertEquals(4, CandidateGrid.pageSize(4, expanded = true, collapseSize = 9))
    }

    @Test
    fun `展开态窗口取前十八个`() {
        val all = (1..45).map { "c$it" }
        val w = CandidateGrid.window(all, 0, all.size, expanded = true, collapseSize = 9)
        assertEquals(18, w.size)
        assertEquals("c1", w.first())
        assertEquals("c18", w.last())
    }

    @Test
    fun `展开态第二页从第十九个开始`() {
        val all = (1..45).map { "c$it" }
        val w = CandidateGrid.window(all, 1, all.size, expanded = true, collapseSize = 9)
        assertEquals("c19", w.first())
    }

    @Test
    fun `页码越界夹回最后一页而不是返回空表`() {
        val all = (1..7).map { "c$it" }
        // 7 个候选、展开态一页 7 个 → 只有一页。页码传 5 应该夹回 0
        val w = CandidateGrid.window(all, 5, all.size, expanded = true, collapseSize = 9)
        assertTrue("越界页码不该返回空表，否则候选条看起来卡死了", w.isNotEmpty())
        assertEquals("c1", w.first())
    }

    @Test
    fun `没有候选时窗口是空的`() {
        assertTrue(CandidateGrid.window(emptyList(), 0, 0, expanded = true, collapseSize = 9).isEmpty())
        assertTrue(CandidateGrid.window(emptyList(), 0, 0, expanded = false, collapseSize = 9).isEmpty())
    }

    @Test
    fun `hasNext 的判定`() {
        // 45 个候选、收起态每页 9 → 第 0 页有下一页
        assertTrue(CandidateGrid.hasNext(45, 0, expanded = false, collapseSize = 9))
        // 最后一页没有
        assertFalse(CandidateGrid.hasNext(9, 0, expanded = false, collapseSize = 9))
        // 展开态 45 个 → 18 一页，第 0 页有
        assertTrue(CandidateGrid.hasNext(45, 0, expanded = true, collapseSize = 9))
        // 展开态 18 个 → 只有一页
        assertFalse(CandidateGrid.hasNext(18, 0, expanded = true, collapseSize = 9))
    }

    @Test
    fun `收起态和展开态切换时首页一致`() {
        val all = (1..45).map { "c$it" }
        val collapsed = CandidateGrid.window(all, 0, all.size, expanded = false, collapseSize = 9)
        val expanded = CandidateGrid.window(all, 0, all.size, expanded = true, collapseSize = 9)
        assertEquals("两种模式的第一页都必须从第一个候选开始", collapsed.first(), expanded.first())
        assertTrue("展开必须比收起显示得多", expanded.size > collapsed.size)
    }

    @Test
    fun `行数只由 PER_ROW 决定，改列数不会让两处算不一致`() {
        for (n in 1..20) {
            val rows = CandidateGrid.rows(n)
            val cnt = CandidateGrid.expandedCount(n)
            assertTrue("$n 个候选算出 $rows 行，装不下 $cnt 个", rows * CandidateGrid.PER_ROW >= cnt)
        }
    }
}
