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

    // ---- 候选条格子数（barSlots）----
    //
    // 这一组盯的是 0.3.22 的真崩溃：按第一个键正常、按第二个键必闪退。
    // 原因是 total 漏算了「展开/收起箭头」，候选数从 1 变 2 以上时多绑一格，
    // getChildAt 返回 null，`as TextView` 抛 NPE。

    @Test
    fun `候选从 1 个变 2 个时格子数要跟着变`() {
        // 第一个键：1 个候选，不出现展开箭头
        val one = CandidateGrid.barSlots(
            assistChip = false, otp = false, hasPrev = false, candidateCount = 1,
            hasNext = false, emptyAssist = false, expandToggle = false, suggestionCount = 0,
        )
        // 第二个键：2 个候选，展开箭头出现 → 必须多一格
        val two = CandidateGrid.barSlots(
            assistChip = false, otp = false, hasPrev = false, candidateCount = 2,
            hasNext = false, emptyAssist = false, expandToggle = true, suggestionCount = 0,
        )
        assertEquals(1, one)
        assertEquals(3, two)
        assertEquals("多出的候选 + 箭头要正好 2 格", one + 2, two)
    }

    @Test
    fun `每个可选格子都要被计入 barSlots`() {
        // 全关：只剩候选本身
        assertEquals(
            5,
            CandidateGrid.barSlots(
                assistChip = false, otp = false, hasPrev = false, candidateCount = 5,
                hasNext = false, emptyAssist = false, expandToggle = false, suggestionCount = 0,
            ),
        )
        // 全开：形1 + 验证码1 + ‹1 + 候选5 + ›1 + 空提示1 + 箭头1 + 分隔符1 + 联想2 = 14
        assertEquals(
            14,
            CandidateGrid.barSlots(
                assistChip = true, otp = true, hasPrev = true, candidateCount = 5,
                hasNext = true, emptyAssist = true, expandToggle = true, suggestionCount = 2,
            ),
        )
    }

    @Test
    fun `没有联想时不占分隔符`() {
        val without = CandidateGrid.barSlots(
            assistChip = false, otp = false, hasPrev = false, candidateCount = 3,
            hasNext = false, emptyAssist = false, expandToggle = true, suggestionCount = 0,
        )
        val with = CandidateGrid.barSlots(
            assistChip = false, otp = false, hasPrev = false, candidateCount = 3,
            hasNext = false, emptyAssist = false, expandToggle = true, suggestionCount = 1,
        )
        assertEquals("1 个联想要占「分隔符 + 自身」两格", without + 2, with)
    }

    @Test
    fun `格子数永远够放下所有要绑的东西`() {
        // 穷举常见组合，确认 barSlots 不小于实际绑定次数。
        for (cand in 0..9) {
            for (sug in 0..3) {
                for (email in 0..4) {
                    for (assist in listOf(false, true)) {
                        for (otp in listOf(false, true)) {
                            for (prev in listOf(false, true)) {
                                for (next in listOf(false, true)) {
                                    for (toggle in listOf(false, true)) {
                                        val slots = CandidateGrid.barSlots(
                                            assist, otp, prev, cand, next, false, toggle, sug, email,
                                        )
                                        var need = cand
                                        if (assist) need++
                                        if (otp) need++
                                        if (prev) need++
                                        if (next) need++
                                        if (toggle) need++
                                        if (sug > 0) need += 1 + sug
                                        if (email > 0) need += 1 + email
                                        assertEquals(
                                            "cand=$cand sug=$sug email=$email assist=$assist " +
                                                "otp=$otp prev=$prev next=$next toggle=$toggle",
                                            need, slots,
                                        )
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /**
     * 邮箱候选要占「1 + n」格，和联想项同一套规则。
     *
     * 漏算这一格就是 0.3.22 那次闪退的翻版：绑定循环多绑一格而容器没扩容，
     * getChildAt 返回 null，`as TextView` 直接 NPE。
     */
    @Test
    fun `邮箱候选占一格加自身数量`() {
        val base = CandidateGrid.barSlots(
            assistChip = false, otp = false, hasPrev = false, candidateCount = 2,
            hasNext = false, emptyAssist = false, expandToggle = false, suggestionCount = 0,
        )
        val withEmail = CandidateGrid.barSlots(
            assistChip = false, otp = false, hasPrev = false, candidateCount = 2,
            hasNext = false, emptyAssist = false, expandToggle = false, suggestionCount = 0,
            emailSuffixCount = 4,
        )
        assertEquals("4 条邮箱候选要占 1 分隔 + 4 自身 = 5 格", base + 5, withEmail)
    }
}
