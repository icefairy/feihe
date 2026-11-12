package com.feihe.ime.keyboard

/**
 * 候选条「展开成多行」的纯计算：一屏能放几行、每行几个、当前页取哪些候选。
 *
 * ## 为什么这段要单独抽出来算
 *
 * 展开后的行/列布局有**一堆互相咬合的边界**：最后一行不满怎么办、候选不够一屏怎么办、
 * 展开态翻页的步长是「行×列」还是不展开时的「候选个数」、切页之后页码要不要归零。
 * 这些错了都不会崩，只会「少显示几个候选」或者「翻页跳过了一批」——真机上根本
 * 注意不到，但用户会觉得候选条不听话。
 *
 * ## 为什么展开态和收起态的分页规则不同
 *
 * - 收起（单行）：一页 = 设置里的「候选个数」（5/7/9），靠 ‹ › 翻页。
 * - 展开（多行）：一页 = 行 × 列，一屏能看十几个，翻页频率大幅下降。
 *
 * 两个模式的 [pageSize] 必须来自同一个地方，否则「展开/收起」切换时候选会跳。
 */
object CandidateGrid {

    /** 展开态最多几行。再多会把键盘挤出去——手机上候选条比键盘还高很荒唐。 */
    const val MAX_ROWS = 3

    /** 展开态每行几个。12 个字的宽度在 1080p 竖屏上正好铺满，不用横滑。 */
    const val PER_ROW = 6

    /**
     * 展开后要显示多少候选。
     *
     * @param total 手上实际有多少候选
     * @return 实际渲染的个数；0 表示没候选（调用方直接把候选条收起来）
     */
    fun expandedCount(total: Int): Int =
        if (total <= 0) 0 else minOf(total, MAX_ROWS * PER_ROW)

    /**
     * 展开后占几行。不足一行也算一行，空候选算 0 行。
     *
     * 这个数直接决定候选条容器的高度（行数 × 行高），所以必须和 [expandedCount]
     * 用同一个 [PER_ROW]，不能各算各的。
     */
    fun rows(total: Int): Int {
        val n = expandedCount(total)
        return if (n == 0) 0 else (n + PER_ROW - 1) / PER_ROW
    }

    /**
     * 展开态一页几个。
     *
     * **行数不足时按实际行数算**：只有 4 个候选时，一页就是 4 个而不是 18 个——
     * 否则 `hasNext` 会算成「还有下一页」，用户点 › 之后候选条瞬间清空。
     */
    fun pageSize(total: Int, expanded: Boolean, collapseSize: Int): Int =
        if (!expanded) collapseSize.coerceAtLeast(1)
        else expandedCount(total).coerceAtLeast(1)

    /**
     * 当前页该渲染的候选窗口。
     *
     * 页码超出范围时**夹回最后一页**而不是返回空表：用户展开后从第 3 页切到只有
     * 1 页的候选时，返回空表会让候选条看起来「卡死了」。
     */
    fun window(candidates: List<String>, page: Int, total: Int, expanded: Boolean, collapseSize: Int): List<String> {
        if (candidates.isEmpty()) return emptyList()
        val size = pageSize(total, expanded, collapseSize)
        val lastPage = ((candidates.size - 1) / size).coerceAtLeast(0)
        val p = page.coerceIn(0, lastPage)
        val start = p * size
        if (start >= candidates.size) return emptyList()
        val end = minOf(start + size, candidates.size)
        return candidates.subList(start, end)
    }

    /**
     * 有没有下一页（展开/收起同一套判定）。
     *
     * **拿 total 比，不要拿 [expandedCount] 比**：expandedCount 是「一页装几个」，
     * 不是「总共有几个」。用它当上限会让 45 个候选展开后算出「只有一页、没有下一页」。
     */
    fun hasNext(total: Int, page: Int, expanded: Boolean, collapseSize: Int): Boolean =
        (page + 1) * pageSize(total, expanded, collapseSize) < total

    /**
     * 收起态候选条里一共有几个格子。
     *
     * 这个数必须和 `FeiheIme.renderCandidates()` 里实际绑定的顺序**逐项对齐**，
     * 否则 `getChildAt` 会取到 null。之前漏算了「展开/收起箭头」：候选从 1 个
     * 变成 2 个以上时多绑一格而容器没扩容，按第二个键直接 NPE 闪退。
     *
     * 抽成纯函数就是为了让这条容易漏的条件链能被单测盯住——它散在渲染代码里，
     * 加一种新格子忘了同步 total，崩的是整个输入法。
     *
     * 绑定顺序（和 renderCandidates 一致）：形 → 验证码 → ‹ → 空提示 → 候选 → › → 箭头 → 分隔符 → 联想
     */
    fun barSlots(
        assistChip: Boolean,
        otp: Boolean,
        hasPrev: Boolean,
        candidateCount: Int,
        hasNext: Boolean,
        emptyAssist: Boolean,
        expandToggle: Boolean,
        suggestionCount: Int,
    ): Int =
        (if (assistChip) 1 else 0) +
            (if (otp) 1 else 0) +
            (if (hasPrev) 1 else 0) +
            candidateCount +
            (if (hasNext) 1 else 0) +
            (if (emptyAssist) 1 else 0) +
            (if (expandToggle) 1 else 0) +
            (if (suggestionCount > 0) 1 + suggestionCount else 0)
}
