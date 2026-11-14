package com.feihe.ime.keyboard

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 形码态键帽字号的契约。
 *
 * 用户原话「可以用小一些的字号」——旧值形码 0.82 / 字母 0.55，两个汉字摊在
 * 约 36dp 宽的键上发胀。这里钉死「整体比普通键小一档」且「提示仍比字母大」，
 * 防止哪天又被顺手放大回去，也防止缩到两行一样大失去主次。
 */
class AssistKeyFaceTest {

    /** 20sp 基值下，形码提示 = 12sp，字母 = 9sp。 */
    @Test
    fun `形码字号常量压到比旧值小一档`() {
        assertTrue(
            "形码提示缩放 ${ASSIST_HINT_SCALE} 没比旧值 0.82 小",
            ASSIST_HINT_SCALE < 0.82f,
        )
        assertTrue(
            "字母缩放 ${ASSIST_LETTER_SCALE} 没比旧值 0.55 小",
            ASSIST_LETTER_SCALE < 0.55f,
        )
    }

    @Test
    fun `形码提示仍比字母大 —— 主次关系不能倒`() {
        assertTrue(
            "形码 ${ASSIST_HINT_SCALE} 应大于字母 ${ASSIST_LETTER_SCALE}",
            ASSIST_HINT_SCALE > ASSIST_LETTER_SCALE,
        )
    }

    /** 钉死具体数值，避免「小一些」被解释成 0.3 那种看不清的程度。 */
    @Test
    fun `具体字号 12sp 与 9sp`() {
        val base = keyTextSizeSp(Key("A", KeyKind.CHAR, "a"))  // 20f
        assertEquals(12f, base * ASSIST_HINT_SCALE, 0.01f)
        assertEquals(9f, base * ASSIST_LETTER_SCALE, 0.01f)
    }

    /** 再小也不能小到看不清：12sp 是汉字可读下限之上。 */
    @Test
    fun `字号不能小到看不清`() {
        val base = keyTextSizeSp(Key("A", KeyKind.CHAR, "a"))
        assertTrue(base * ASSIST_HINT_SCALE >= 11f)
        assertTrue(base * ASSIST_LETTER_SCALE >= 8f)
    }

    /**
     * 键面提示必须是单行——`makeAssistKey` 里给 hintView 设了 isSingleLine，
     * 否则两个部件在最窄键上换行会把 46dp 键撑高、和字母行叠在一起。
     * 静态断言源码，避免有人重构时把那行删掉。
     */
    @Test
    fun `形码提示必须单行显示`() {
        val src = java.io.File(
            "src/main/java/com/feihe/ime/keyboard/KeyboardView.kt"
        ).readText()
        val body = src.substringAfter("val hintView = TextView(ctx).apply {")
            .substringBefore("layoutParams =")
        assertTrue("hintView 必须 isSingleLine = true", body.contains("isSingleLine = true"))
    }
}
