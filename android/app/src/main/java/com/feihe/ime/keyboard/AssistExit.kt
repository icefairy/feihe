package com.feihe.ime.keyboard

/**
 * 形码态退出的纯判据。
 *
 * 形码态把 26 个字母键换成「字母小字 + 部件提示大字」的双层 [FrameLayout]，
 * 这**不是**普通键帽上的一个文字字段，改状态位不会让键盘自己变回去——必须
 * `rebuildKeyboard()` 重新 build 一次键盘区，才能回归「字母 + 上滑标点」的布局。
 *
 * 用户原话：「形码开启后 选了确定下来的字后 键盘上的形码还在 应该要自动收掉，
 * 回归之前字母和标点符号的键盘布局」。根因就是上屏路径只把 [assistActive]
 * 置了 false，没重建键盘 View。
 *
 * 抽成纯函数是为了让「什么时候该重建」有唯一裁决点，且能被普通 JVM 单测钉死
 * （[com.feihe.ime.FeiheIme.resetAssist] 依赖 Android View，单测测不到）。
 */
object AssistExit {

    /**
     * 是否要在退出形码态时重建键盘。
     *
     * 只有「本来开着、现在关掉」这一次跳变才需要重建：
     * - `wasActive = false`：普通打字本来就是字母键，选词后不必重建（避免每选一个词
     *   都抖一下，也避免无谓地 removeAllViews + 重新 measure/layout）。
     * - `wasActive = true`：键帽现在还是双层形码结构，必须重建回字母/标点布局。
     *
     * @param wasActive 调用清理之前 [assistActive] 的值
     */
    fun shouldRebuildKeyboard(wasActive: Boolean): Boolean = wasActive
}
