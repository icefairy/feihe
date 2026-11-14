package com.feihe.ime.keyboard

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * 形码态「选字上屏后自动收掉、键盘回归字母/标点布局」的契约。
 *
 * 用户原话：「形码开启后 选了确定下来的字后 键盘上的形码还在 应该要自动收掉，
 * 回归之前字母和标点符号的键盘布局」。
 *
 * 根因：上屏路径（commitWord 等）把 assistActive 置了 false，但形码键帽是另一个
 * View 结构，不重建键盘就还留在屏幕上。判据抽成 [AssistExit.shouldRebuildKeyboard]
 * 这个纯函数，测试直接调它；生产侧「resetAssist 真的按它重建」「上屏真的调了
 * resetAssist」用源码静态断言钉死（这两处依赖 Android View，普通 JVM 单测测不到）。
 */
class AssistExitTest {

    /** 本来开着形码 → 退出时必须重建键盘，否则键帽还是双层部件提示。 */
    @Test
    fun `形码态退出必须重建键盘`() {
        assertTrue(AssistExit.shouldRebuildKeyboard(wasActive = true))
    }

    /** 本来没开形码 → 普通选词不该重建，避免每选一个词都抖一下。 */
    @Test
    fun `非形码态选词不重建键盘`() {
        assertFalse(AssistExit.shouldRebuildKeyboard(wasActive = false))
    }

    /** resetAssist 必须真的用这个判据来重建，不能只把状态位清了就算完。 */
    @Test
    fun `resetAssist 退出时调 rebuildKeyboard`() {
        val body = imeSrc().substringAfter("private fun resetAssist() {")
            .substringBefore("\n    }")
        assertTrue(
            "resetAssist 必须记下退出前的 assistActive 并传给判据",
            body.contains("wasActive") && body.contains("AssistExit.shouldRebuildKeyboard"),
        )
        assertTrue(
            "resetAssist 判据为真时必须 rebuildKeyboard()，否则键盘上的形码不会消失",
            body.contains("rebuildKeyboard()"),
        )
    }

    /**
     * 上屏汇合点必须走 resetAssist（而不是自己手写 assistActive=false）。
     *
     * commitWord 是点候选 / 空格选首 / 分段整句上屏的共同终点，漏了它用户点选字
     * 就收不掉形码。把这条撤掉，测试应当变红。
     */
    @Test
    fun `commitWord 上屏必须调 resetAssist`() {
        val body = imeSrc().substringAfter("private fun commitWord(")
            .substringBefore("\n    }")
        assertTrue(
            "commitWord 必须调 resetAssist() 才能在选字上屏后收掉形码态",
            body.contains("resetAssist()"),
        )
    }

    /** 退格删空形码、分段选完字，也都收敛到 resetAssist，不再散落手写重建。 */
    @Test
    fun `其余退出路径不再手写 assistActive_false_加_rebuildKeyboard`() {
        val src = imeSrc()
        // 除了 resetAssist 自身，函数体里不该再有连续两行
        // 「assistActive = false」+「rebuildKeyboard()」的手写退出（会和统一逻辑漂）。
        val handWritten = Regex(
            """assistActive\s*=\s*false\s*\n\s*rebuildKeyboard\(\)"""
        )
        // resetAssist 里是 assistActive = false 后隔了几行才 rebuild，不会被上面命中；
        // 但保险起见先把 resetAssist 函数体挖掉再找。
        val withoutReset = run {
            val i = src.indexOf("private fun resetAssist() {")
            val j = src.indexOf("/** 点「形」", i)
            if (i < 0 || j < 0) src else src.substring(0, i) + src.substring(j)
        }
        assertFalse(
            "还有路径手写 assistActive=false 紧跟 rebuildKeyboard()，应统一走 resetAssist()",
            handWritten.containsMatchIn(withoutReset),
        )
    }

    private fun imeSrc() = File("src/main/java/com/feihe/ime/FeiheIme.kt").readText()
}
