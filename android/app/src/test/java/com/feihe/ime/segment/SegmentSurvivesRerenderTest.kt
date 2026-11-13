package com.feihe.ime.segment

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 分段态在「同一串码重绘」时必须原样保留。
 *
 * ## 钉的是哪个 bug
 *
 * 用户报：组词时选最后一个字，点「展开候选」，前面选的全没了，
 * 候选条退回整串字母（`lmhdilvi`），再按一下就把它直接上屏。
 *
 * 根因不在 `SegmentState`（它是不可变 data class，`copy` 必然保留已选），
 * 而在宿主层：展开候选会把键盘设成 GONE，宿主重新布局并重入
 * `onStartInputView()` → `refreshCandidates()` → `refreshSegmentMode()`，
 * 而后者当时是**无条件** `segmentState = null`。
 *
 * 于是「同一个 pending 重新渲染一次」就自毁了。
 *
 * 修法：分段态记录它建立时对应的那串码（`segmentStateCode`），
 * 码没变 = 重绘，保留；码变了 = 用户改了输入，才丢弃。
 *
 * 这个文件用纯状态模型把两条判据钉住（宿主层的行为靠 Instrumented 测试，
 * 但「码相同保留 / 码不同丢弃」这个决策本身是纯逻辑，JVM 就能验）。
 */
class SegmentSurvivesRerenderTest {

    /** 复刻 refreshSegmentMode 的判据。 */
    private fun shouldDrop(segmentStateOrNull: Any?, segmentStateCode: String, pending: String): Boolean {
        if (segmentStateOrNull == null) return false
        return segmentStateCode != pending
    }

    @Test
    fun `同一串码重绘 —— 分段态必须保留`() {
        val code = "lmhdilvi"
        // 展开候选 / 收起候选 / 窗口尺寸变化 都会让宿主重发 onStartInputView，
        // pending 一个字都没变。
        assertEquals(false, shouldDrop(SegmentState(emptyList()), code, code))
    }

    @Test
    fun `码变了 —— 分段态必须丢弃`() {
        // 用户按了退格或输入了新字母：旧的段已经对不上新码，必须丢。
        assertEquals(true, shouldDrop(SegmentState(emptyList()), "lmhdilvi", "lmhdilv"))
        assertEquals(true, shouldDrop(SegmentState(emptyList()), "lmhdilvi", "lmhdilviq"))
    }

    @Test
    fun `本来就没有分段态 —— 不需要做什么`() {
        assertEquals(false, shouldDrop(null, "", "lmhdilvi"))
    }

    @Test
    fun `SegmentState 选择后已选的字不会丢`() {
        // 补一层保险：确认 SegmentState 自己是对的。
        // 三段的候选，逐段选不同的字，前两段的选择必须一直保留。
        val s0 = SegmentState(
            segments = listOf(listOf("李", "里", "理"), listOf("明", "名"), listOf("远", "元")),
            codes = listOf("li", "mk", "yr"),
        )
        val s1 = s0.open(0).choose(2)                       // 选「理」→ 自动展开第 2 段
        val s2 = s1.choose(1)                                // 选「名」→ 自动展开第 3 段
        val s3 = s2.choose(0)                                // 选「远」→ 最后一段，focus=-1
        assertEquals("理名远", s3.sentence())
        assertEquals(-1, s3.focus)
        // 中途展开/收起不该改动已选
        assertEquals("理名远", s3.back().open(1).sentence())
    }

    @Test
    fun `最后一段选完后 sentence 仍然是完整整句`() {
        // 「选最后一个字时点展开」那个场景，即使 focus 已经 -1，
        // sentence() 也必须能拼出三段的当前选择（否则会退化成上屏字母串）。
        val s = SegmentState(
            segments = listOf(listOf("李"), listOf("明"), listOf("远")),
            selected = listOf(0, 0, 0),
            focus = -1,
            codes = listOf("li", "mk", "yr"),
        )
        assertEquals("李明远", s.sentence())
        assertTrue(s.sentenceCode().isNotEmpty())
    }
}
