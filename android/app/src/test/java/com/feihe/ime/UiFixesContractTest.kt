package com.feihe.ime

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * 2026-11-14 一轮「手机版问题」里纯 UI 改动的回归闸门。
 *
 * 这些改动全是 Android View 层（面板高度、字形、显隐、返回键、按钮字符），
 * JVM 单测起不了 InputMethodService，沿用项目里 VisualContractTest / AssistExitTest
 * 的做法：**静态读源码断言契约**。改动被误删/回退时当场变红。
 *
 * 工作目录是 android/app，路径从 src/ 开始（PathProbeTest 实测）。
 */
class UiFixesContractTest {

    private fun src(name: String) = File("src/main/java/com/feihe/ime/$name").readText()

    // ---- 问题3：剪贴板面板限高 + 清空按钮 ----

    @Test
    fun `剪贴板面板不得用 weight 撑满全屏`() {
        val all = src("FeiheIme.kt")
        val panelBlock = all
            .substringAfter("clipPanel = LinearLayout(this)")
            .substringBefore("showPage(Page.LETTERS)")
        // weight=1 会把面板撑到吃掉整个输入法窗口（多条历史时铺满全屏）。
        // 必须是显式 height，且在 openClipboard 里按键盘实测高度写入。
        assertFalse(
            "clipPanel 的 root.addView 不能再带 weight=1f（会占满全屏）",
            Regex("root\\.addView\\(\\s*clipPanel[\\s\\S]{0,400}?1f,").containsMatchIn(panelBlock),
        )
        // openClipboard 在面板定义之前，得从全文里找。
        assertTrue(
            "openClipboard 必须按 keyboardHost.height 给面板钉死高度",
            all.contains("khH > 0") && all.contains("lp.height = panelH"),
        )
    }

    @Test
    fun `剪贴板面板必须有清空历史按钮且真正清空`() {
        val body = src("FeiheIme.kt")
        assertTrue("标题行要有清空按钮", body.contains("清空剪贴板历史"))
        assertTrue("清空按钮要绑 clearAllClips", body.contains("setOnClickListener { clearAllClips() }"))
        val clear = body.substringAfter("private fun clearAllClips()").substringBefore("private fun renderClipboard()")
        assertTrue("clearAllClips 要清空 clipEntries", clear.contains("clipEntries = emptyList()"))
        assertTrue("clearAllClips 要写盘", clear.contains("persistClips()"))
    }

    // ---- 问题4：收起键盘按钮纵向居中 ----

    @Test
    fun `收起键盘按钮换成字形居中的实心下三角`() {
        val body = src("FeiheIme.kt")
            .substringAfter("hideButton = iconButton(")
            .substringBefore("statusBar = LinearLayout")
        assertTrue(
            "收起按钮不能再用 ⌄（墨量偏下，看着不居中），要用 ▾",
            body.contains("\"▾\"") && !body.contains("\"⌄\""),
        )
        assertTrue("要显式保证 gravity 居中", body.contains("gravity = Gravity.CENTER"))
    }

    // ---- 问题7：候选栏不显示表情图标 ----

    @Test
    fun `候选栏 emojiChip 必须为 false 且不再绑定表情格`() {
        val render = src("FeiheIme.kt")
            .substringAfter("val expandToggle = candidates.size > 1")
            .substringBefore("candidateScroll?.visibility = View.VISIBLE")
        assertTrue("emojiChip 必须是 false", Regex("val emojiChip = false").containsMatchIn(render))
        assertFalse(
            "候选栏不能再绑定 emojiChip（打字时不该有表情图标）",
            render.contains("bindEmojiChip(it)"),
        )
    }

    // ---- 问题8：表情入口点一下展开再点收起 ----

    @Test
    fun `状态条表情按钮走 toggle 而不是单向 showPage`() {
        val body = src("FeiheIme.kt")
        assertTrue(
            "emojiButton 点击要调 toggleEmojiPage",
            body.substringAfter("emojiButton = iconButton(")
                .substringBefore("hideButton =").contains("toggleEmojiPage()"),
        )
        val toggle = body.substringAfter("private fun toggleEmojiPage()")
            .substringBefore("private fun openSettings()")
        assertTrue(
            "toggle 要用 page==EMOJI 判据，收起回 LETTERS",
            toggle.contains("page == Page.EMOJI") &&
                toggle.contains("Page.LETTERS") && toggle.contains("Page.EMOJI"),
        )
    }

    // ---- 问题1：繁体输出（Android 注册 converter + 上屏过 apply）----

    @Test
    fun `Android 必须注册简繁转换器且上屏点过 convert`() {
        val jni = File("../jni/src/lib.rs").readText()
        assertTrue(
            "initNative 必须 set_converter(zhconv_android::to_traditional)",
            jni.contains("feihe_core::zhconv::set_converter(zhconv_android::to_traditional)"),
        )
        assertTrue("必须有 convertTextNative 入口", jni.contains("convertTextNative"))
        assertTrue("convertTextNative 要复用 core 的 apply", jni.contains("feihe_core::zhconv::apply(&input)"))
        // 表数据必须是自含文件，不是空壳
        val mod = File("../jni/src/zhconv_android.rs").readText()
        assertTrue("zhconv_android 要 include s2t 表", mod.contains("include_str!(\"s2t.txt\")"))
        val s2t = File("../jni/src/s2t.txt").readText()
        assertTrue("s2t 表要有「设→設」", s2t.contains("设\t設"))
        assertTrue("s2t 表规模应上千", s2t.lines().count { "\t" in it } > 2000)
        // Kotlin 侧：三个中文词上屏点都要过 convert，且 learn/record 仍用简体原文
        val ime = src("FeiheIme.kt")
        val ne = File("src/main/java/com/feihe/ime/engine/NativeEngine.kt").readText()
        assertTrue("NativeEngine 要声明 convertTextNative", ne.contains("external fun convertTextNative"))
        assertTrue("NativeEngine 要有 convert() 包装", ne.contains("fun convert(text: String)"))
        val cnt = Regex("ic\\.commitText\\(NativeEngine\\.convert\\(word\\), 1\\)").findAll(ime).count()
        assertTrue("commitWord/Suggestion/Quick 三处上屏都要过 convert（实际 $cnt 处）", cnt == 3)
        // 学习口径不能跟着转：record/learn 仍传 word
        assertTrue("record 仍用简体 word", ime.contains("NativeEngine.record(prev, word)"))
        assertTrue("learn 仍用简体 word", ime.contains("NativeEngine.learn(code, word)"))
    }

    // ---- 问题2：configNative 必须输出 assist_mode ----

    @Test
    fun `configNative 必须把 assist_mode 放进快照串`() {
        // 根因：设置页把 assist_mode 写进了 config.ini，但输入法进程读的是
        // configNative() 拼的快照串；以前这串漏了 assist_mode，NativeEngine.settings()
        // 就永远走默认 "xiaohe" → 选了自然码键盘仍显示小鹤部件。
        val jni = File("../jni/src/lib.rs").readText()
        val cfgFn = jni.substringAfter("fn Java_com_feihe_ime_engine_NativeEngine_configNative")
            .substringBefore("make_string(&mut env, &s)")
        assertTrue("configNative 的串里必须有 assist_mode=", cfgFn.contains("assist_mode={}"))
        assertTrue("值要来自 c.assist_mode.key()", cfgFn.contains("c.assist_mode.key()"))
    }

    // ---- 问题10：英文模式长按字母键直接输大写 ----

    @Test
    fun `字母键长按要在 withSwipe 里自判长持并回调 onLongKey`() {
        val kv = File("src/main/java/com/feihe/ime/keyboard/KeyboardView.kt").readText()
        val swipe = kv.substringAfter("private fun withSwipe(").substringBefore("private fun keyBackground(")
        assertTrue("withSwipe 要接 onLongKey", swipe.contains("onLongKey: (Key) -> Unit"))
        assertTrue("要自己 postDelayed 判长持", swipe.contains("CHAR_LONG_PRESS_MS") && swipe.contains("postDelayed(longRunnable"))
        assertTrue("长按到点要回调 onLongKey", swipe.contains("onLongKey(key)"))
        assertTrue("上滑要取消长按", swipe.contains("removeCallbacks(longRunnable)"))
        assertTrue("长处理后 UP 不能再补点击", swipe.contains("longHandled"))
        val ime = src("FeiheIme.kt")
        val longKey = ime.substringAfter("private fun onLongKey(key: Key)").substringBefore("if (key.kind == KeyKind.SPACE)")
        assertTrue(
            "onLongKey 要判 CHAR+英文态并输出大写",
            longKey.contains("KeyKind.CHAR && !chinese") &&
                longKey.contains("commitText(key.code.uppercase())"),
        )
    }

    // ---- 问题9：返回键先收面板再收键盘 ----

    @Test
    fun `onKeyDown 要在 clipOpen 时先收剪贴板面板`() {
        val body = src("FeiheIme.kt")
        assertTrue("必须 override onKeyDown", body.contains("override fun onKeyDown(keyCode: Int, event: KeyEvent)"))
        val keydown = body.substringAfter("override fun onKeyDown(keyCode: Int, event: KeyEvent)")
            .substringBefore("override fun onCreateInputView")
        assertTrue("要拦 KEYCODE_BACK", keydown.contains("KeyEvent.KEYCODE_BACK"))
        assertTrue("要判 clipOpen", keydown.contains("clipOpen"))
        assertTrue("要先 closeClipboard()", keydown.contains("closeClipboard()"))
        assertTrue("要消费掉这次返回键 return true", keydown.contains("return true"))
        assertTrue("非面板态交回 super", keydown.contains("super.onKeyDown(keyCode, event)"))
    }

    // ---- 问题10：英文态「先打字母再上滑/点符号」把字母重复一遍（m → mm.） ----

    /** 取某个 `KeyKind.X ->` 分支到分支结尾之间的源码（下一个分支 或 该分支的收尾大括号）。 */
    private fun keyBranch(all: String, marker: String): String {
        val after = all.substringAfter(marker)
        // 最后一个分支（NAV）后面没有下一个 KeyKind，要退回到同级缩进的收尾 "}"
        val next = after.indexOf("\n            KeyKind.")
        val end = if (next >= 0) next else after.indexOf("\n            }")
        return if (end >= 0) after.substring(0, end) else after
    }

    @Test
    fun `英文态上屏字面字符不得把已上屏字母重打一遍`() {
        val ime = src("FeiheIme.kt")

        // 前提：英文是逐字母真上屏，pending 只是「已进宿主那串字母」的账本。
        // 一旦这条变了，下面所有断言的前提也就变了。
        assertTrue(
            "英文上屏必须是增量 commitText(text.takeLast(1))，否则账本语义不成立",
            ime.contains("ic.commitText(text.takeLast(1), 1)"),
        )

        // 三个「字面字符」分支（标点/符号数字/光标跳转）都不得再自己判 commitSegmentIfActive，
        // 必须走统一的 flushBeforeLiteral()：它会在英文态只清账本，不回打。
        for (marker in listOf("KeyKind.PUNCT ->", "KeyKind.TEXT ->", "KeyKind.NAV ->")) {
            val b = keyBranch(ime, marker)
            assertFalse(
                "$marker 不得自己调 commitSegmentIfActive（英文态会漏掉，导致重复上屏）",
                b.contains("commitSegmentIfActive()"),
            )
            assertTrue("$marker 必须走 flushBeforeLiteral()", b.contains("flushBeforeLiteral()"))
        }

        // 收尾函数本身必须按中英分叉：英文只清账本，中文才提交。
        val helper = ime.substringAfter("private fun flushBeforeLiteral()")
            .substringBefore("\n    private fun clearComposing()")
        assertTrue("flushBeforeLiteral 要判英文态", helper.contains("if (!chinese)"))
        assertTrue("英文态要调 commitEnglishComposing()", helper.contains("commitEnglishComposing()"))
        assertTrue("中文态才 commitPending()", helper.contains("commitPending()"))
        // 英文分支必须在 commitPending 之前就 return，不能落到下一行。
        assertTrue(
            "英文分支必须提前 return，不能继续走到 commitPending()",
            Regex("if \\(!chinese\\) \\{\\s*commitEnglishComposing\\(\\)\\s*return\\s*\\}")
                .containsMatchIn(helper),
        )
    }
}
