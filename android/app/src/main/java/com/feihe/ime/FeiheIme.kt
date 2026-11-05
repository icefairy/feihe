package com.feihe.ime

import android.inputmethodservice.InputMethodService
import android.util.Log
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.inputmethod.EditorInfo
import android.widget.LinearLayout
import android.widget.TextView
import com.feihe.ime.engine.NativeEngine
import com.feihe.ime.keyboard.Key
import com.feihe.ime.keyboard.KeyKind
import com.feihe.ime.keyboard.KeyboardView
import com.feihe.ime.keyboard.QwertyLayout

/**
 * 飞鹤输入法 · Android 壳（M2 阶段）。
 *
 * 现在是一个**能打字的英文态输入法 + 26 键中文键盘**：字母先攒在组合串里
 * （输入框里能看到下划线），空格 / 标点 / 换行时才上屏。
 *
 * ## 为什么先做英文态
 *
 * 中文态的链路是「字母 → 引擎（双拼解码 + 词库）→ 候选 → 选词上屏」。引擎
 * （`feihe-core` 交叉编译出来的 `libfeihe_android.so`）已经接上了，但**词库还没搬
 * 进 core**，所以现在只接了编码能力：状态条上能看到全拼到双拼的转换结果。
 *
 * 但**键盘、组合串、上屏、删除**这些平台层的东西不需要引擎就能做完、也能验。
 * 所以先把平台层做扎实：引擎接进来时只换「字母怎么变成候选」这一步，
 * 那是一个接口的事，不是重写。
 */
class FeiheIme : InputMethodService() {

    /** 正在组合的字母（还没上屏）。英文态下它就是要上屏的内容本身。 */
    private val pending = StringBuilder()

    /** 中/英态。中文态要等引擎接进来才真正生效，现在只影响状态条显示。 */
    private var chinese = true

    private var statusText: TextView? = null

    override fun onCreate() {
        super.onCreate()
        Log.i(TAG, "onCreate: 输入法服务已创建")
    }

    override fun onStartInput(info: EditorInfo?, restarting: Boolean) {
        super.onStartInput(info, restarting)
        pending.setLength(0)
        // 密码框这一档以后要拦（不学习、不记忆、不联想）。骨架期先记日志，
        // 让日志能证明「我们看得到 inputType」——这是所有按输入框分派的前提。
        Log.i(
            TAG,
            "onStartInput: inputType=0x${(info?.inputType ?: 0).toString(16)} " +
                "restarting=$restarting fieldId=${info?.fieldId}"
        )
    }

    override fun onStartInputView(info: EditorInfo?, restarting: Boolean) {
        super.onStartInputView(info, restarting)
        refreshStatus()
    }

    /**
     * 关掉全屏编辑模式。
     *
     * 系统默认在「横屏 + 输入框小」时把整块屏幕接管过去给我们画，那时键盘和候选条
     * 的位置全乱。输入法要的是**贴着屏幕底部的一条**，不是全屏。
     */
    override fun onEvaluateFullscreenMode(): Boolean = false

    override fun onCreateInputView(): View {
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
        }

        statusText = TextView(this).apply {
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 12f)
            gravity = Gravity.CENTER_VERTICAL
            setPadding(dp(12f), dp(6f), dp(12f), dp(6f))
            setBackgroundColor(0xFFEDEEF0.toInt())
        }
        root.addView(
            statusText,
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.WRAP_CONTENT,
            ),
        )

        root.addView(KeyboardView.build(this, QwertyLayout.rows, ::onKey))
        refreshStatus()
        return root
    }

    private fun onKey(key: Key) {
        when (key.kind) {
            KeyKind.CHAR -> {
                pending.append(key.code)
                pushComposing()
            }

            KeyKind.DELETE -> {
                if (pending.isNotEmpty()) {
                    pending.setLength(pending.length - 1)
                    pushComposing()
                } else {
                    // 组合串空了才删文档里的字。反过来（先删文档）会把用户打了一半的
                    // 拼音吃掉，这是最容易被骂的行为之一。
                    currentInputConnection?.deleteSurroundingText(1, 0)
                }
            }

            KeyKind.SPACE -> {
                if (pending.isEmpty()) {
                    commitText(" ")
                } else {
                    // 英文态：空格就是把组合串原样上屏。
                    // 接了引擎之后这里会变成「选第一个候选」。
                    commitPending()
                }
            }

            KeyKind.PUNCT -> {
                commitPending()
                commitText(key.code)
            }

            KeyKind.ENTER -> {
                commitPending()
                commitText("\n")
            }

            KeyKind.MODE -> {
                chinese = !chinese
                refreshStatus()
            }

            // Shift / 符号页还没实现，先什么都不做——不要假装支持
            KeyKind.SHIFT, KeyKind.SYMBOL -> Unit
        }
    }

    /** 把组合串同步进输入框（带下划线）。空串就是结束组合。 */
    private fun pushComposing() {
        val ic = currentInputConnection ?: return
        ic.setComposingText(pending.toString(), 1)
        refreshStatus()
    }

    private fun commitPending() {
        if (pending.isEmpty()) return
        val ic = currentInputConnection ?: return
        // commitText 会替换掉正在组合的区域，不需要先 finishComposingText；
        // 先 finish 再 commit 在部分宿主里会多出一次光标移动。
        ic.commitText(pending.toString(), 1)
        pending.setLength(0)
        refreshStatus()
    }

    private fun commitText(text: CharSequence) {
        currentInputConnection?.commitText(text, 1)
    }

    private fun refreshStatus() {
        statusText?.text = buildString {
            append(if (chinese) "中" else "英")
            append(" · 26 键")
            if (pending.isNotEmpty()) {
                append(" · ")
                append(pending)
                // 把双拼码也显示出来——这是**屏幕上第一处真正来自 Rust 引擎的东西**。
                // 打 `nihao` 时这里应该出现 `nihc`；如果一直不出现，说明 .so 没加载
                // 或者符号名对不上（看 logcat 里的 FeiheIme 标签）。
                val code = if (NativeEngine.isSyllable(pending.toString())) {
                    NativeEngine.encodePinyin(pending.toString())
                } else {
                    ""
                }
                if (code.isNotEmpty()) append("→").append(code)
            }
            append("  ·  引擎 ")
            append(NativeEngine.coreVersion())
        }
    }

    override fun onFinishInput() {
        pending.setLength(0)
        super.onFinishInput()
    }

    private fun dp(v: Float): Int = (v * resources.displayMetrics.density + 0.5f).toInt()

    companion object {
        private const val TAG = "FeiheIme"
    }
}
