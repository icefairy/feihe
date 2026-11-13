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
        val per = CandidateGrid.PER_ROW
        assertEquals(0, CandidateGrid.rows(0))
        assertEquals(1, CandidateGrid.rows(1))
        assertEquals(1, CandidateGrid.rows(per))
        assertEquals(2, CandidateGrid.rows(per + 1))
        assertEquals(3, CandidateGrid.rows(per * 3))
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
        val perPage = CandidateGrid.PER_ROW * CandidateGrid.MAX_ROWS
        assertEquals(perPage, CandidateGrid.pageSize(45, expanded = true, collapseSize = 9))
        // 只有 4 个候选时，一页就是 4 个——否则 hasNext 会误报「还有下一页」
        assertEquals(4, CandidateGrid.pageSize(4, expanded = true, collapseSize = 9))
    }

    @Test
    fun `展开态窗口取满一页`() {
        val all = (1..45).map { "c$it" }
        val perPage = CandidateGrid.PER_ROW * CandidateGrid.MAX_ROWS
        val w = CandidateGrid.window(all, 0, all.size, expanded = true, collapseSize = 9)
        assertEquals(perPage, w.size)
        assertEquals("c1", w.first())
        assertEquals("c$perPage", w.last())
    }

    @Test
    fun `展开态第二页紧接第一页`() {
        val all = (1..45).map { "c$it" }
        val perPage = CandidateGrid.PER_ROW * CandidateGrid.MAX_ROWS
        val w = CandidateGrid.window(all, 1, all.size, expanded = true, collapseSize = 9)
        assertEquals("c${perPage + 1}", w.first())
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
        // 展开态 45 个 → 一页 = 行×列，第 0 页有
        assertTrue(CandidateGrid.hasNext(45, 0, expanded = true, collapseSize = 9))
        // 展开态刚好一页 → 没有下一页
        val perPage = CandidateGrid.PER_ROW * CandidateGrid.MAX_ROWS
        assertFalse(CandidateGrid.hasNext(perPage, 0, expanded = true, collapseSize = 9))
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

    // ── 展开态格子宽度：两字词不能折行 ────────────────────────────────
    //
    // 用户报的现象：展开候选后，两字词「智慧」显示成上下两行（上面智、下面慧），
    // 六个格子各自竖排，整片候选区看起来像乱码，还误以为候选词重复了。
    //
    // 根因是每个格子 = 屏宽/6 且左右 padding 各 14dp，可用宽度不够两个字。
    // 修法是把格内 padding 收到 6dp 并强制单行；这组测试钉住「收完之后装得下」。

    @Test
    fun `展开态格子宽度要装得下两字词`() {
        // 360dp 屏 - 容器左右各 8dp(共16) = 344 可用 / 4 列 = 86dp，
        // 再扣格内左右各 6dp(共12) = 74dp 给文字。
        assertEquals(
            "344/4=86，减 12 → 74dp",
            74,
            CandidateGrid.cellWidthDp(screenWidthDp = 360, horizontalPaddingDp = 16, cellPaddingDp = 12),
        )
    }

    @Test
    fun `两字词在 360dp 屏上不折行`() {
        // 只放真正的两字词（vihv 的实际候选里两字的是这四个）
        val words = listOf("只会", "智慧", "指挥", "知会")
        assertFalse(
            "两字词 40dp < 74dp 可用，不应折行",
            CandidateGrid.willTruncate(words, screenWidthDp = 360),
        )
    }

    @Test
    fun `vihv 的完整候选里最长三字词只省略不折行`() {
        // 真实候选：只会/智慧/指挥/知会/指挥官/指挥部/智慧的/指挥员/指挥下/止回阀
        val real = listOf(
            "只会", "智慧", "指挥", "知会", "指挥官",
            "指挥部", "智慧的", "指挥员", "指挥下", "止回阀",
        )
        // 4 列后三字词也装得下，整组候选都不该被截断
        assertFalse(
            "vihv 的全部候选（最长三字）都应完整显示，不该省略",
            CandidateGrid.willTruncate(real, screenWidthDp = 360),
        )
    }

    @Test
    fun `三字词在 360dp 屏上完整显示不被截断`() {
        // 这是把 PER_ROW 从 6 改成 4 的直接原因：6 列时三字词被截成「指…」
        assertFalse(
            "三字词 60dp < 74dp 可用，必须完整显示",
            CandidateGrid.willTruncate(listOf("指挥官", "止回阀", "智慧的"), screenWidthDp = 360),
        )
    }

    @Test
    fun `旧的 14dp 格内内边距会挤坏两字词`() {
        // 回归防线：如果谁把格内 padding 改回 28dp（左右各 14），这条会红
        assertTrue(
            "格内 28dp 时可用只有 58dp，两字词能装下；但三字词 60dp 装不下",
            CandidateGrid.willTruncate(
                listOf("指挥官"),
                screenWidthDp = 360,
                cellPaddingDp = 28,
            ),
        )
    }

    // ── 收起态分页：修「打字卡顿」的核心契约 ──────────────────────

    @Test
    fun `收起态一屏放得下的个数按屏宽算`() {
        // 392dp：两字词 40dp + padding 12dp = 52dp 一格 → 7 格，
        // 但要留 2 格给「展开」「›」操作键 → 5 个候选，夹在 3~5 内
        assertEquals(5, CandidateGrid.collapseSlotCount(screenWidthDp = 392))
        // 360dp：6 格 − 2 = 4
        assertEquals(4, CandidateGrid.collapseSlotCount(screenWidthDp = 360))
        // 320dp：6 格 − 2 = 4
        assertEquals(4, CandidateGrid.collapseSlotCount(screenWidthDp = 320))
    }

    @Test
    fun `收起态格子数有上下限`() {
        // 极窄屏不能只显示 1 个候选
        assertEquals(3, CandidateGrid.collapseSlotCount(screenWidthDp = 60))
        // 宽屏也不铺太多：收起态只是「快速挑一个」，全量交给展开
        assertEquals(5, CandidateGrid.collapseSlotCount(screenWidthDp = 2000))
        // 非法输入也要给个能用的值
        assertEquals(3, CandidateGrid.collapseSlotCount(screenWidthDp = 0))
    }

    @Test
    fun `系统字号放大时收起态格子数减少`() {
        // 字号 1.5 倍：一格 (20*1.5)*2 + 12 = 72dp → 360/72 = 5
        val big = CandidateGrid.collapseSlotCount(screenWidthDp = 360, textSizeDp = 30f)
        val normal = CandidateGrid.collapseSlotCount(screenWidthDp = 360, textSizeDp = 20f)
        assertTrue("字号放大后格子数应变少（$normal → $big）", big < normal)
    }

    /**
     * **卡顿修复的核心断言**：281 个候选，收起态一次只能渲染一页的量，
     * 绝不能因为「候选很多」就把 281 个 View 全建出来。
     */
    @Test
    fun `281 个候选收起态只渲染一页`() {
        val many = (1..281).map { "候选$it" }
        val slots = CandidateGrid.collapseSlotCount(screenWidthDp = 392)
        val w = CandidateGrid.window(many, 0, many.size, expanded = false, collapseSize = slots)
        assertEquals("一页最多渲染 $slots 个", slots, w.size)
        assertTrue("绝不能把 281 个全渲染出来（那是 170ms 卡顿的根因）", w.size < 20)
    }

    /** 候选总数不受影响：还能翻到最后一页的最后一个。 */
    @Test
    fun `无限候选靠翻页仍能全部取到`() {
        val many = (1..281).map { "候选$it" }
        val slots = CandidateGrid.collapseSlotCount(screenWidthDp = 392)
        val lastPage = (many.size - 1) / slots
        val last = CandidateGrid.window(many, lastPage, many.size, expanded = false, collapseSize = slots)
        assertEquals("最后一页的最后一个必须是第 281 个", "候选281", last.last())
        assertTrue("281 个候选确实有 $lastPage 页，翻页是必需的", lastPage >= 30)
    }

    /** 翻页不能漏也不能重：把所有页拼起来必须等于原列表。 */
    @Test
    fun `翻页拼接后与原始候选完全一致`() {
        val many = (1..281).map { "候选$it" }
        val slots = CandidateGrid.collapseSlotCount(screenWidthDp = 360)
        val pages = (many.size + slots - 1) / slots
        val rebuilt = (0 until pages).flatMap { p ->
            CandidateGrid.window(many, p, many.size, expanded = false, collapseSize = slots)
        }
        assertEquals("翻完所有页应还原出全部 281 个，不能漏或重", many, rebuilt)
    }

    /** hasNext 必须跟着新页大小走，否则「›」会消失或点了没反应。 */
    @Test
    fun `收起态 281 个候选有下一页`() {
        val slots = CandidateGrid.collapseSlotCount(screenWidthDp = 392)
        assertTrue(CandidateGrid.hasNext(281, 0, expanded = false, collapseSize = slots))
        val lastPage = (281 - 1) / slots
        assertFalse("最后一页不该再有下一页", CandidateGrid.hasNext(281, lastPage, expanded = false, collapseSize = slots))
    }

    // ── 列数自适应：换屏 / 系统字号都不能把三字词挤掉 ──────────────

    @Test
    fun `360dp 屏算 4 列`() {
        // 344 可用 / 72 每格 = 4.7 → 4 列。6 列时三字词会被截成「指…」
        assertEquals(4, CandidateGrid.columnsFor(screenWidthDp = 360))
    }

    @Test
    fun `窄屏 320dp 也算 4 列而不是挤成 3 列`() {
        // 304 / 72 = 4.2 → 4。每格 76dp，三字词 72dp 刚好
        assertEquals(4, CandidateGrid.columnsFor(screenWidthDp = 320))
    }

    @Test
    fun `大屏平板自动加列`() {
        // 800 - 16 = 784 / 72 = 10.8 → 夹到 MAX_COLS=6
        assertEquals(6, CandidateGrid.columnsFor(screenWidthDp = 800))
        // 500 - 16 = 484 / 72 = 6.7 → 仍夹到 6
        assertEquals(6, CandidateGrid.columnsFor(screenWidthDp = 500))
    }

    @Test
    fun `超小屏也至少排 2 列`() {
        assertEquals(2, CandidateGrid.columnsFor(screenWidthDp = 120))
        assertEquals(2, CandidateGrid.columnsFor(screenWidthDp = 0))
    }

    @Test
    fun `系统字号放大后列数要减少`() {
        // 字号 1.3 倍时三字词需 78dp+12=90dp，344/90 = 3.8 → 3 列
        assertEquals(3, CandidateGrid.columnsFor(screenWidthDp = 360, fontScale = 1.3f))
        // 字号 1.5 倍：344/102 = 3.4 → 3
        assertEquals(3, CandidateGrid.columnsFor(screenWidthDp = 360, fontScale = 1.5f))
        // 字号 2.0（超大）：344/132 = 2.6 → 2
        assertEquals(2, CandidateGrid.columnsFor(screenWidthDp = 360, fontScale = 2.0f))
    }

    @Test
    fun `算出的列数必须保证三字词装得下`() {
        // 不变式：任何屏宽 / 任何字号下，按算出的列数分格都要装得下三字词
        for (w in listOf(320, 360, 400, 480, 600, 800)) {
            for (fs in listOf(1.0f, 1.15f, 1.3f, 1.5f)) {
                val cols = CandidateGrid.columnsFor(w, fontScale = fs)
                val cell = CandidateGrid.cellWidthDp(w, cellPaddingDp = CandidateGrid.CELL_PADDING_DP)
                val perCell = (w - CandidateGrid.CONTAINER_PADDING_DP) / cols - CandidateGrid.CELL_PADDING_DP
                val triple = (20f * 3f * fs).toInt()
                assertTrue(
                    "width=$w fontScale=$fs cols=$cols 每格 ${perCell}dp 装不下三字词 ${triple}dp（cellWidthDp=$cell）",
                    perCell >= triple,
                )
            }
        }
    }
}
