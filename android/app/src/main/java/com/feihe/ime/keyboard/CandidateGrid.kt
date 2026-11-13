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

    /**
     * 展开态默认最多几行。
     *
     * 这个值只在**没告诉它能用多高**时兜底用。实际渲染时由调用方按可用高度算出
     * 真实行数传进来（见 [expandedCount] 的 maxRows 参数）——展开态会把键盘区
     * 整块收起来，那时候能铺七八行，写死 3 行等于白白浪费一半屏幕。
     */
    const val MAX_ROWS = 3

    /**
     * 展开态每行几个。**运行时按屏幕宽度自适应**（见 [columnsFor]）。
     *
     * 以前是 `const val = 6`，导致 360dp 屏上每格只有 57dp、扣掉格内 padding
     * 剩 45dp，三字词（指挥官 / 止回阀 / 智慧的，20sp 需 60dp）被截成「指…」。
     * 改成 4 又会在平板/大屏上浪费一整排空间 —— 写死任何数字都不对，
     * 因为用户的屏宽、系统字号都可能不同。
     *
     * 现在由 [FeiheIme.updateColumns] 在创建输入视图和配置变化时按真实屏宽算，
     * 保证「三字词一定装得下」这个不变式在各种屏幕上成立。
     *
     * **只在主线程读写**（输入视图只有一份，渲染与配置回调都在主线程）。
     */
    @Volatile
    var PER_ROW = 4

    /** 折叠态/展开态共用的最小列数（再窄也要能并排看两个候选）。 */
    const val MIN_COLS = 2

    /** 最大列数：再宽也不铺太多，一行超过 6 个手指点不准。 */
    const val MAX_COLS = 6

    /**
     * 一个格子至少要留多少宽度（dp），才能完整显示「三字候选词」。
     *
     * 三字 × 20sp = 60dp 文字 + 格内左右 padding 12dp = 72dp。
     * 低于这个宽度就必须靠 ellipsize 截断，用户看到「指…」等于没有候选。
     */
    const val MIN_CELL_DP = 72

    /**
     * 按屏幕宽度算出该用几列。
     *
     * 不变式：每格可用宽度 >= 三字词宽度（默认 60dp），这样最常见的三字候选
     * 一定能完整显示；同时列数夹在 [MIN_COLS]..[MAX_COLS] 之间，窄屏不至于
     * 挤成一列，宽屏也不至于一行铺七八个点到隔壁。
     *
     * @param screenWidthDp 屏幕可用宽度（dp，已扣系统栏）
     * @param fontScale 系统字号缩放（`scaledDensity / density`）；用户把字号
     *   调大时汉字实际占宽同步变大，列数必须跟着减，否则又会被截断
     * @param textSizeDp 候选字号（默认 20sp）
     * @param containerPaddingDp 候选行容器的左右内边距之和
     * @param cellPaddingDp 单个格子的左右内边距之和
     */
    fun columnsFor(
        screenWidthDp: Int,
        fontScale: Float = 1f,
        textSizeDp: Float = 20f,
        containerPaddingDp: Int = CONTAINER_PADDING_DP,
        cellPaddingDp: Int = CELL_PADDING_DP,
    ): Int {
        if (screenWidthDp <= 0) return MIN_COLS
        val usable = screenWidthDp - containerPaddingDp
        if (usable <= 0) return MIN_COLS
        // 三字词在系统字号放大后需要的实际宽度
        val scale = if (fontScale > 0f) fontScale else 1f
        val triple = (textSizeDp * 3f * scale + cellPaddingDp).toInt()
        val need = maxOf(triple, MIN_CELL_DP)
        val cols = usable / need
        return cols.coerceIn(MIN_COLS, MAX_COLS)
    }

    /**
     * 展开后要显示多少候选。
     *
     * @param total 手上实际有多少候选
     * @param maxRows 这次实际能放几行。默认用 [MAX_ROWS]（不接管键盘时的保守值）
     * @return 实际渲染的个数；0 表示没候选（调用方直接把候选条收起来）
     */
    fun expandedCount(total: Int, maxRows: Int = MAX_ROWS): Int {
        if (total <= 0) return 0
        val rows = if (maxRows <= 0) MAX_ROWS else maxRows
        return minOf(total, rows * PER_ROW)
    }

    /**
     * 展开后占几行。不足一行也算一行，空候选算 0 行。
     *
     * 这个数直接决定候选条容器的高度（行数 × 行高），所以必须和 [expandedCount]
     * 用同一个 [PER_ROW]，不能各算各的。
     */
    fun rows(total: Int, maxRows: Int = MAX_ROWS): Int {
        val n = expandedCount(total, maxRows)
        return if (n == 0) 0 else (n + PER_ROW - 1) / PER_ROW
    }

    /**
     * 展开态一页几个。
     *
     * **行数不足时按实际行数算**：只有 4 个候选时，一页就是 4 个而不是 18 个——
     * 否则 `hasNext` 会算成「还有下一页」，用户点 › 之后候选条瞬间清空。
     */
    fun pageSize(
        total: Int,
        expanded: Boolean,
        collapseSize: Int,
        maxRows: Int = MAX_ROWS,
    ): Int =
        if (!expanded) collapseSize.coerceAtLeast(1)
        else expandedCount(total, maxRows).coerceAtLeast(1)

    /**
     * 当前页该渲染的候选窗口。
     *
     * 页码超出范围时**夹回最后一页**而不是返回空表：用户展开后从第 3 页切到只有
     * 1 页的候选时，返回空表会让候选条看起来「卡死了」。
     */
    fun window(
        candidates: List<String>,
        page: Int,
        total: Int,
        expanded: Boolean,
        collapseSize: Int,
        maxRows: Int = MAX_ROWS,
    ): List<String> {
        if (candidates.isEmpty()) return emptyList()
        val size = pageSize(total, expanded, collapseSize, maxRows)
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
    fun hasNext(
        total: Int,
        page: Int,
        expanded: Boolean,
        collapseSize: Int,
        maxRows: Int = MAX_ROWS,
    ): Boolean =
        (page + 1) * pageSize(total, expanded, collapseSize, maxRows) < total

    /**
     * 展开态每个格子能分到多少宽度（dp）。
     *
     * 用来判断「最长的候选词能不能装进一格」。装不进就会折行——两字词竖着
     * 排成上下两行，整片候选区看起来像乱码（用户报的「两字词汇展开候选显示有问题」）。
     *
     * ## 为什么 cellPaddingDp 默认值要跟渲染代码绑定
     *
     * 默认值就是 `bindCandidateView` 里 `setPadding(dp(x),0,dp(x),0)` 的 x×2。
     * 两处必须一致，否则测试会「绿着但线上折行」——那正是这个 bug 逃过测试的原因。
     * 改渲染 padding 时这里必须同步，测试里有断言盯着这个等式。
     */
    const val CELL_PADDING_DP = 12  // = bindCandidateView 的 6dp × 2
    const val CONTAINER_PADDING_DP = 16  // = 行容器 setPadding(dp(8f)) × 2

    fun cellWidthDp(
        screenWidthDp: Int,
        horizontalPaddingDp: Int = CONTAINER_PADDING_DP,
        cellPaddingDp: Int = CELL_PADDING_DP,
    ): Int {
        if (screenWidthDp <= 0) return 0
        val usable = screenWidthDp - horizontalPaddingDp
        if (usable <= 0) return 0
        return (usable / PER_ROW) - cellPaddingDp
    }

    /**
     * 这组候选在展开态会不会折行（即最长那个词装不进一格）。
     *
     * 汉字宽度约等于字号（20sp ≈ 20dp），所以「字数 × 20dp」就是它需要的宽度。
     * 只在测试和诊断用——正式渲染靠 `maxLines=1` 硬保证不折行，
     * 但这个判据能提前告诉我们「格子太窄，字会被省略成 智… 而不是正常显示」。
     */
    fun willTruncate(
        words: List<String>,
        screenWidthDp: Int,
        textSizeDp: Int = 20,
        horizontalPaddingDp: Int = CONTAINER_PADDING_DP,
        cellPaddingDp: Int = CELL_PADDING_DP,
    ): Boolean {
        if (words.isEmpty()) return false
        val avail = cellWidthDp(screenWidthDp, horizontalPaddingDp, cellPaddingDp)
        if (avail <= 0) return true
        val worst = words.maxOf { w -> w.codePointCount(0, w.length) * textSizeDp }
        return worst > avail
    }

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
        emailSuffixCount: Int = 0,
    ): Int =
        (if (assistChip) 1 else 0) +
            (if (otp) 1 else 0) +
            (if (hasPrev) 1 else 0) +
            candidateCount +
            (if (hasNext) 1 else 0) +
            (if (emptyAssist) 1 else 0) +
            (if (expandToggle) 1 else 0) +
            (if (suggestionCount > 0) 1 + suggestionCount else 0) +
            (if (emailSuffixCount > 0) 1 + emailSuffixCount else 0)
}
