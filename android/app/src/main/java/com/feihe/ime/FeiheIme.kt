package com.feihe.ime

import android.inputmethodservice.InputMethodService
import android.util.Log
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.inputmethod.EditorInfo
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView

/**
 * 飞鹤输入法 · Android 骨架。
 *
 * 这个类**故意**只做四件事：被系统叫起来、画出一个键盘、往输入框塞字、删字。
 *
 * 目的是把平台假设一次验完，而不是先写两个月引擎再发现"候选条弹不出来"：
 *
 * 1. 装得上、系统设置里能看到、能切过来（BIND_INPUT_METHOD + method.xml + subtype）
 * 2. `onCreateInputView` 画出来的东西真的出现在屏幕上（而不是一片空白 / 被导航栏盖住）
 * 3. `currentInputConnection.commitText` 真的进了输入框（不同宿主的 IC 行为不一致，
 *    这是最容易"看起来对但没上屏"的地方）
 * 4. `deleteSurroundingText` 能在各种输入框里正常删字
 *
 * 验证通过之后，`onCreateInputView` 里那块按钮换成真键盘，上屏逻辑换成
 * `feihe-core` 出的候选——那时候才有"引擎"这个词的位置。
 */
class FeiheIme : InputMethodService() {

    /** 上一次宿主给的输入类型，用来确认 onStartInput 真的被调了（骨架期只记日志）。 */
    private var lastInputType: Int = 0

    override fun onCreate() {
        super.onCreate()
        Log.i(TAG, "onCreate: 输入法服务已创建")
    }

    override fun onStartInput(info: EditorInfo?, restarting: Boolean) {
        super.onStartInput(info, restarting)
        lastInputType = info?.inputType ?: 0
        // 密码框这一档以后要拦（不学习、不记忆、不联想）。骨架期先记下来，
        // 让日志能证明「我们看得到 inputType」——这是所有按输入框分派的前提。
        Log.i(
            TAG,
            "onStartInput: inputType=0x${lastInputType.toString(16)} " +
                "restarting=$restarting fieldId=${info?.fieldId}"
        )
    }

    override fun onStartInputView(info: EditorInfo?, restarting: Boolean) {
        super.onStartInputView(info, restarting)
        // 真机上「三键导航 vs 手势导航」的高度差会在这里暴露出来：键盘高度由
        // 系统给的 window 决定，切换导航方式后必须重新量。骨架期先只记一条。
        Log.i(TAG, "onStartInputView: 键盘视图已显示")
    }

    override fun onCreateInputView(): View {
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER_HORIZONTAL
            // 底色用浅灰：一眼能看出「这是我们的视图」而不是宿主自己的键盘。
            setBackgroundColor(0xFFEDEDED.toInt())
        }

        root.addView(
            TextView(this).apply {
                text = getString(R.string.skeleton_title)
                textSize = 14f
                setPadding(0, 24, 0, 4)
            }
        )

        root.addView(
            TextView(this).apply {
                text = getString(R.string.skeleton_hint)
                textSize = 12f
                setPadding(24, 0, 24, 16)
            }
        )

        val row = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER
        }

        row.addView(
            Button(this).apply {
                text = getString(R.string.key_hello)
                setOnClickListener { commit("你好") }
            },
            LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f)
        )

        row.addView(
            Button(this).apply {
                text = getString(R.string.key_backspace)
                setOnClickListener { backspace() }
            },
            LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f)
        )

        root.addView(row)
        return root
    }

    /**
     * 上屏。
     *
     * 注意 `commitText` 的第二个参数是 newCursorPosition，填 1 表示「光标放到
     * 文字末尾」。填 0 或负数在部分宿主里会把光标丢到奇怪的位置——这是骨架期
     * 就要固定下来的约定，以后所有上屏都走这一个出口。
     */
    private fun commit(text: CharSequence) {
        val ic = currentInputConnection
        if (ic == null) {
            Log.w(TAG, "commit: currentInputConnection 为空（宿主还没给输入连接）")
            return
        }
        ic.commitText(text, 1)
    }

    /**
     * 删一个字。
     *
     * `deleteSurroundingText(1, 0)` 删光标前 1 个 **char**。注意中文里一个汉字是
     * 1 个 char、emoji 常常是 2 个（代理对），骨架期先按 char 删，后面接引擎时
     * 再按「一个候选 / 一个音节」的粒度删——那是完全不同的语义。
     */
    private fun backspace() {
        val ic = currentInputConnection ?: return
        ic.deleteSurroundingText(1, 0)
    }

    override fun onFinishInput() {
        super.onFinishInput()
        lastInputType = 0
    }

    companion object {
        private const val TAG = "FeiheIme"
    }
}
