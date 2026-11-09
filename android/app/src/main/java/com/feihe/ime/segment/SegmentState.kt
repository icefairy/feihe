package com.feihe.ime.segment

/**
 * 逐段选择的纯状态（A1）。
 *
 * ## 手势决定：点段换词，不做左右移段
 *
 * 桌面端分词态是「左右移段 + 退格拆段」，但手机候选条只有一行，左右移段要用户
 * 先在脑子里维护一个看不见的游标，很别扭（`docs/后续待办.md` A1 原话）。
 * 这里改成**直接点要改的那一段**：候选条画成 `[张三][李]` 一排 chip，点哪个就展开
 * 哪个的候选，选完自动回到 chip 视图。没有游标，也就没有「我现在在第几段」的问题。
 *
 * 分段规则不在这里 —— 在 `feihe_core::segment::plan`，和桌面端同一份。
 * 这个类只管「每段选了第几个」，所以能跑普通 JVM 单测。
 *
 * @param segments 每段的候选列表（顺序即词频顺序，来自引擎，界面不重排）
 * @param selected 每段当前选中的下标，默认全 0
 * @param focus 当前展开的是哪一段；-1 = chip 视图
 */
data class SegmentState(
    val segments: List<List<String>>,
    val selected: List<Int> = List(segments.size) { 0 },
    val focus: Int = -1,
) {
    /** 是否值得进分段态：至少两段、每段都有候选。 */
    val active: Boolean
        get() = segments.size >= 2 && segments.all { it.isNotEmpty() }

    /** 默认整句：每段取当前选中的那个词拼起来。 */
    fun sentence(): String = buildString {
        for (i in segments.indices) {
            append(segments[i][selected[i]])
        }
    }

    /** 展开第 i 段。下标越界或该段没候选就原样返回。 */
    fun open(index: Int): SegmentState {
        if (index !in segments.indices || segments[index].isEmpty()) return this
        return copy(focus = index)
    }

    /** 收起展开的段，回到 chip 视图。 */
    fun back(): SegmentState = copy(focus = -1)

    /** 当前展开段的候选；没展开时是空表。 */
    fun focusedWords(): List<String> =
        if (focus in segments.indices) segments[focus] else emptyList()

    /**
     * 在展开段里选第 [index] 个候选，更新该段并回到 chip 视图。
     * 没展开 / 下标越界时原样返回——点空白处不该改动任何已选的词。
     */
    fun choose(index: Int): SegmentState {
        if (focus !in segments.indices) return this
        if (index !in segments[focus].indices) return this
        return copy(
            selected = selected.toMutableList().also { it[focus] = index },
            focus = -1,
        )
    }

    /**
     * 按**词**选（形码态用）。
     *
     * 形码筛过之后，界面上是筛过的子集，下标和 [segments] 对不上；用词本身去定位
     * 才不会选错（未筛选的列表仍走 [choose]）。
     *
     * [keepOpen] = 选完自动展开**下一段**：形码态跨段保持，用户不用再点一下。
     * 已经是最后一段时回到 chip 视图（focus = -1）。
     */
    fun chooseWord(word: String, keepOpen: Boolean = false): SegmentState {
        if (focus !in segments.indices) return this
        val i = segments[focus].indexOf(word)
        if (i < 0) return this
        val next = if (keepOpen && focus + 1 < segments.size && segments[focus + 1].isNotEmpty()) {
            focus + 1
        } else {
            -1
        }
        return copy(selected = selected.toMutableList().also { it[focus] = i }, focus = next)
    }

    /** chip 视图里第 i 段显示的词。 */
    fun wordAt(index: Int): String =
        if (index in segments.indices && selected[index] in segments[index].indices) {
            segments[index][selected[index]]
        } else {
            ""
        }
}
