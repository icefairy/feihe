package com.feihe.ime.keyboard

/**
 * 候选「展开态」该不该继续保持。
 *
 * ## 为什么这个判断值得单独抽出来
 *
 * 展开态不是普通的 UI 偏好——它是把**整个键盘藏起来**、借它那块高度来铺候选。
 * 所以一旦手上没有可画的东西还继续撑着，用户看到的是一块**空白面板**外加一根
 * 没人想点的「收起」条，键盘还回不来，得手动再点一次。
 *
 * 这个洞在两条路径上各犯过一次：
 *   1. 分段逐字选完最后一个字 → 自动上屏，但展开态没跟着收
 *   2. 普通拼音展开候选 → 点选一个词 → 候选清空，展开态照旧
 *
 * 两次都是「数据清了、呈现标志没清」。所以判据不能再散落在调用点上，
 * 而是收敛成这一处纯逻辑，由 [keepsExpanded] 统一回答。
 *
 * ## 判据
 *
 * 「有没有东西可画」——有候选（当前页窗口非空）或者正停在某一段，就该展开；
 * 两者都没有，展开态没有任何存在理由。
 */
object ExpandedGate {

    /**
     * @param windowSize 当前页该渲染的候选个数
     * @param segmentFocus 分段态当前展开的段下标；不在分段态或回到 chip 视图时传 -1
     * @param segmentCount 分段态总段数（用来判断 focus 是否真的有效）
     */
    fun keepsExpanded(windowSize: Int, segmentFocus: Int, segmentCount: Int): Boolean {
        if (windowSize > 0) return true
        // 正在某一段：即使形码把候选筛空，操作行（‹ 返回 / 形）还在，
        // 用户需要那个面板找到退路
        return segmentFocus >= 0 && segmentFocus < segmentCount
    }
}
