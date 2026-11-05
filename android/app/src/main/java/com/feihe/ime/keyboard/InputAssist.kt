package com.feihe.ime.keyboard

/**
 * 上档状态机 + 数字选词映射。
 *
 * ## 为什么单独抽成纯 Kotlin
 *
 * 这两块逻辑都不碰 Android API，但**特别容易在边界上写错**：上档「一次性 / 锁定」
 * 的区别只在「打完这个字母之后要不要落下」，数字选词则是「第几个候选」的 off-by-one。
 * 放进 `FeiheIme`（InputMethodService 子类）就没法跑普通 JVM 单测了，只能上真机
 * 靠手点——而这两处错了在真机上的表现都是「偶尔不对」，最难复现。
 *
 * 和 `KeyboardLayout` 一样：真值放纯数据里，界面只负责画。
 */

/** 上档的三种状态。`ONCE` 是老行为（按一下、下一个字母大写、打完落下）。 */
enum class ShiftState {
    /** 不激活。 */
    OFF,

    /** 一次性：下一个字母大写，之后自动落回 [OFF]。 */
    ONCE,

    /** 锁定：连续大写，直到用户再点一次 ⇧ 或切中英/换页。 */
    LOCKED,
}

object ShiftMachine {
    /**
     * 单击 ⇧。
     *
     * - [ShiftState.OFF] → [ShiftState.ONCE]：进入一次性大写。
     * - [ShiftState.ONCE] / [ShiftState.LOCKED] → [ShiftState.OFF]：再点一下就是取消，
     *   这比「ONCE 点第二下变 LOCKED」直观得多，也避免双击手势的时序歧义。
     */
    fun onTap(cur: ShiftState): ShiftState = when (cur) {
        ShiftState.OFF -> ShiftState.ONCE
        ShiftState.ONCE, ShiftState.LOCKED -> ShiftState.OFF
    }

    /**
     * 长按 ⇧ = 锁定/解锁。
     *
     * **故意不用双击**：键盘里 ⌫ 已经用了「长按连续删」，长按是这套键盘既有的手势；
     * 而双击会和「连按两下」（连打两个大写、或连删两个字）抢同一个时间窗口，
     * 阈值调松了误触、调紧了按不出来。见 `docs/后续待办.md` A3。
     */
    fun onLongPress(cur: ShiftState): ShiftState =
        if (cur == ShiftState.LOCKED) ShiftState.OFF else ShiftState.LOCKED

    /**
     * 打完一个字母之后。只有 [ShiftState.ONCE] 会落下；[ShiftState.LOCKED] 保持。
     *
     * 切中英 / 换页 / 上屏标点这些"上下文断开"的动作不调它，由调用方直接置 [ShiftState.OFF]
     * ——那些地方本来就该把上档一起清掉。
     */
    fun afterChar(cur: ShiftState): ShiftState =
        if (cur == ShiftState.ONCE) ShiftState.OFF else cur

    /** 这个字母要不要大写。锁定态下每个字母都大写。 */
    fun upper(cur: ShiftState): Boolean = cur != ShiftState.OFF
}

object NumberSelect {
    /** 候选最多给 9 个，和桌面端 `1`–`9` 选词一致。 */
    const val MAX = 9

    /**
     * 当前该显示哪些数字键：`1..min(候选数, 9)`。
     *
     * 没候选时返回空表 → 整行隐藏（**不常驻**），这正是 A2 里「做成可隐藏」的意思：
     * 它只在组合态占一行高度，键盘收起来后字母键一dp不少。
     */
    fun digits(candidateCount: Int): IntRange =
        if (candidateCount <= 0) IntRange.EMPTY else 1..minOf(candidateCount, MAX)

    /**
     * 数字键 → 候选下标（0-based）。越界返回 null，调用方直接忽略。
     *
     * 单独抽出来是因为这里必然有个 `-1`，写进 `onKey` 里就是那种"看起来对、
     * 真机上点 1 选了第二个"的 bug。
     */
    fun indexOf(digit: Int, candidateCount: Int): Int? {
        if (digit !in 1..MAX || digit > candidateCount) return null
        return digit - 1
    }
}
