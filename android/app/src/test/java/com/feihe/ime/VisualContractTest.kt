package com.feihe.ime

import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * 盯住「只能靠看图才能发现的视觉 bug」。
 *
 * 这类问题不会崩溃、不打异常日志，只会让用户点不到按钮、或者看不见按钮，
 * 却极难复现。所以把踩过的坑钉成静态断言。
 *
 * 注意工作目录是 `android/app`（不是 `android/`），所以路径从 `src/` 开始。
 * 这一点用 PathProbeTest 实测确认过。
 */
class VisualContractTest {

    private fun src(name: String) = File("src/main/java/com/feihe/ime/$name").readText()

    @Test
    fun `两套配色的控件描边都必须与候选条背景不同色`() {
        val src = src("Palette.kt")
        val light = src.substringAfter("private val LIGHT = P(").substringBefore("\n    )")
        val dark = src.substringAfter("private val DARK = P(").substringBefore("\n    )")

        fun grab(block: String, key: String): Long? =
            Regex("$key\\s*=\\s*0x([0-9A-Fa-f]{8})").find(block)
                ?.groupValues?.get(1)?.toLong(16)

        for ((name, block) in listOf("LIGHT" to light, "DARK" to dark)) {
            val bg = grab(block, "keyboardBg")
            val chip = grab(block, "chipStroke")
            assertTrue("$name 缺 keyboardBg", bg != null)
            assertTrue("$name 缺 chipStroke", chip != null)
            assertNotEquals(
                "$name: chipStroke 与 keyboardBg 同色 → 描边画在候选条上等于隐形。" +
                    "浅色模式曾经两者都是 0xFFD9DCE1，用户反复反馈「按钮没有边框」，" +
                    "而实际是画了看不见。",
                bg, chip,
            )
        }
    }

    @Test
    fun `形按钮必须显式给 LayoutParams 宽度`() {
        val body = src("FeiheIme.kt")
            .substringAfter("private fun bindAssistView")
            .substringBefore("private fun bindSeparatorView")
        assertTrue(
            "bindAssistView 必须显式设置 layoutParams 宽度 48dp。" +
                "根因：HorizontalScrollView 里的 LinearLayout 只采纳 LayoutParams 宽度，" +
                "minWidth/minimumWidth 被忽略 → 实测 w=0 → 按钮点不到。" +
                "用户报的「容易点不到」就是这个。",
            body.contains("LinearLayout.LayoutParams(dp("),
        )
    }

    @Test
    fun `形按钮与展开按钮都必须有背景`() {
        val full = src("FeiheIme.kt")
        for (fn in listOf("bindAssistView", "bindExpandToggle")) {
            val body = full.substringAfter("private fun $fn").take(2500)
            assertTrue(
                "$fn 必须设置 background，否则用户看不出这是一颗按钮",
                body.contains("GradientDrawable()"),
            )
        }
    }

    @Test
    fun `收起候选胶囊必须用 chipStroke 描边`() {
        val full = src("FeiheIme.kt")
        val i = full.indexOf("▴ 收起候选")
        assertTrue("找不到收起候选文案", i > 0)
        val around = full.substring((i - 1200).coerceAtLeast(0), i)
        assertTrue("收起条必须 setStroke", around.contains("setStroke("))
        assertTrue(
            "收起条描边必须用 chipStroke（与背景有对比的那个），" +
                "原来用 stroke(0xFF9AA0A8) 偏淡、又有 setColor(TRANSPARENT) 压制绘制",
            around.contains("chipStroke"),
        )
    }
}
