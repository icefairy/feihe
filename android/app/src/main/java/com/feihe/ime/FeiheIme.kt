package com.feihe.ime

import android.inputmethodservice.InputMethodService
import android.util.Log
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.inputmethod.EditorInfo
import android.widget.HorizontalScrollView
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
 * 现在的状态：**26 键 + 小鹤双拼 + 真候选**。
 *
 * 打 `nihao` 时引擎把它编成双拼码 `nihc`，拿这个码去查词库，候选摆在键盘上方一条
 * 里，点一下上屏；空格选第一个。
 *
 * ## 这一版刻意没做的（别以为是漏了）
 *
 * - **分词态**（打一长串中文码 `nihcshij` 再切段逐段选）：Windows 端有，手机上
 *   交互太重（要左右移段）。**全拼多音节是支持的**（`zhongguo` → `vsgo` → 「中国」），
 *   不支持的只是「已经输成双拼码的长串」。
 * - **数字选词**：键盘没有数字行，加一行会挤掉字母键的高度。选词靠点。
 * - **联想浮窗**：有联想，但**不弹独立窗口**（那会压住键盘、还会把候选条顶跳）。
 *   联想词接在候选条末尾，用 `→` 前缀区分，点一下上屏。
 * - **Shift / 符号页**：没实现就不假装支持。
 */
class FeiheIme : InputMethodService() {

    /** 正在组合的字母（还没上屏）。中文态下它是双拼码的「原始输入」。 */
    private val pending = StringBuilder()

    /** 当前候选。空表示没有候选（码还没成型，或者词库里没有）。 */
    private var candidates: List<String> = emptyList()

    /**
     * 联想词（上一个词的后续）。和 [candidates] **分开存**，因为它们不参与同一套
     * 选择语义：空格只会选候选的第一个，绝不会选联想；联想只能用手指点。
     *
     * 混在一个列表里会让「按空格」这个肌肉记忆变得不可预测。
     */
    private var suggestions: List<String> = emptyList()

    /**
     * 上一个**上屏的词**，联想基于它。
     *
     * 只在「连续打词」时有值：打完标点、空格、回车，或者删掉了文档里的字，都清空
     * ——那些动作已经把上下文断开了，再拿旧词联想就是猜。
     */
    private var prevWord: String = ""

    /** 中/英态。 */
    private var chinese = true

    private var statusText: TextView? = null
    private var candidateBar: LinearLayout? = null
    private var candidateScroll: HorizontalScrollView? = null

    override fun onCreate() {
        super.onCreate()
        // 数据目录用 filesDir：输入法进程随时可能被杀，配置和词频都必须落盘到
        // 应用私有目录（不是缓存目录——缓存会被系统清掉）。
        NativeEngine.init(filesDir.absolutePath)
        Log.i(TAG, "onCreate: 引擎 ${NativeEngine.coreVersion()}")
    }

    override fun onStartInput(info: EditorInfo?, restarting: Boolean) {
        super.onStartInput(info, restarting)
        clearComposing()
        // 换了输入框（甚至有可能是换了 App），上一个词跟这里没有任何关系。
        // 不在这里清，切到微信输入框时会拿着半小时前在便签里打的词去联想。
        prevWord = ""
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

        // 候选条：一条横向可滚动的行。**不能是 Dialog**——输入法窗口层级特殊，
        // Dialog 会被系统盖住；它必须是 InputView 的一部分。
        candidateBar = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(dp(8f), 0, dp(8f), 0)
        }
        candidateScroll = HorizontalScrollView(this).apply {
            isHorizontalScrollBarEnabled = false
            addView(candidateBar)
        }
        root.addView(
            candidateScroll,
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                dp(40f),
            ),
        )

        root.addView(KeyboardView.build(this, QwertyLayout.rows, ::onKey))
        refreshStatus()
        refreshCandidates()
        return root
    }

    private fun onKey(key: Key) {
        when (key.kind) {
            KeyKind.CHAR -> {
                // 一开始打新东西，旧联想就必须消失：留着它等于把「几分钟前的上下文」
                // 又摆在手指下面，点错了上屏的词完全不搭。
                suggestions = emptyList()
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
                    // 删掉了一个字，上一个词是什么已经说不清了
                    forgetContext()
                }
            }

            KeyKind.SPACE -> {
                if (pending.isEmpty()) {
                    commitText(" ")
                    // 空格断开上下文：下一个词和上一个词之间没有直接关系
                    forgetContext()
                } else if (chinese && candidates.isNotEmpty()) {
                    // 中文态空格 = 选第一个。这是所有拼音输入法的肌肉记忆，
                    // 不这么做用户会以为候选是装饰品。
                    commitWord(candidates.first())
                } else {
                    // 没有候选（码还没成词，或者这是个英文串）→ 原样上屏。
                    // **不能什么都不做**：那样用户打了一串字母就卡住，只能一个个删。
                    commitPending()
                }
            }

            KeyKind.PUNCT -> {
                // 有候选就先上屏第一个，再粘标点——否则会出现「nihc，」这种
                // 拼音和标点混在一起的串。
                // `thenSuggest = false`：标点之后不联想（「你好，」后面接什么都对，
                // 联想只会给出噪音），Windows 端也是这么定的。
                if (chinese && candidates.isNotEmpty()) {
                    commitWord(candidates.first(), thenSuggest = false)
                } else {
                    commitPending()
                }
                commitText(key.code)
                forgetContext()
            }

            KeyKind.ENTER -> {
                commitPending()
                commitText("\n")
                forgetContext()
            }

            KeyKind.MODE -> {
                chinese = !chinese
                // 切英文时把中文候选丢掉：那批候选是按中文码查的，留着只会误导
                forgetContext()
                refreshCandidates()
            }

            // Shift / 符号页还没实现，先什么都不做——不要假装支持
            KeyKind.SHIFT, KeyKind.SYMBOL -> Unit
        }
    }

    /** 把组合串同步进输入框（带下划线），然后刷新候选。 */
    private fun pushComposing() {
        val ic = currentInputConnection ?: return
        ic.setComposingText(pending.toString(), 1)
        refreshCandidates()
        refreshStatus()
    }

    /**
     * 重新查一遍候选。
     *
     * 中文态下把已输入的字母交给引擎编码（引擎自己切音节），再拿码去查词库。
     * 编码失败（还没打成完整音节，比如 `nih`）就没有候选，但**不清空已输入的
     * 字母**——用户还在打。
     *
     * 限制：`nihao` → `nihc` 能出「你好」，但 `nihcshij` 这种**已经输中文码**的串
     * 还不行——用户必须一直打全拼。这个差异在类注释里写明了。
     */
    private fun refreshCandidates() {
        candidates = if (!chinese || pending.isEmpty()) {
            emptyList()
        } else {
            val code = NativeEngine.encodePhrase(pending.toString())
            if (code.isEmpty()) emptyList() else NativeEngine.lookup(code, 9)
        }
        renderCandidates()
    }

    private fun renderCandidates() {
        val bar = candidateBar ?: return
        bar.removeAllViews()
        for ((i, word) in candidates.withIndex()) {
            val item = TextView(this).apply {
                text = word
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 20f)
                setPadding(dp(14f), 0, dp(14f), 0)
                // 第一个候选高亮：它对应空格，得让人看出来
                setTextColor(if (i == 0) 0xFF1A6FD4.toInt() else 0xFF202124.toInt())
                setOnClickListener { commitWord(word) }
            }
            bar.addView(item)
        }

        // 联想接在候选后面。**加 `→` 前缀不是为了好看**：它必须让人一眼看出
        // 「这个不是候选，点了会直接上屏、不吃空格」，否则用户会以为空格能选它。
        if (suggestions.isNotEmpty()) {
            if (candidates.isNotEmpty()) {
                bar.addView(
                    TextView(this).apply {
                        text = "|"
                        setTextSize(TypedValue.COMPLEX_UNIT_SP, 16f)
                        setTextColor(0xFFBDBDBD.toInt())
                        setPadding(dp(2f), 0, dp(2f), 0)
                    },
                )
            }
            for (word in suggestions) {
                bar.addView(
                    TextView(this).apply {
                        text = "→$word"
                        setTextSize(TypedValue.COMPLEX_UNIT_SP, 20f)
                        setPadding(dp(14f), 0, dp(14f), 0)
                        setTextColor(0xFF5F6368.toInt())
                        setOnClickListener { commitSuggestion(word) }
                    },
                )
            }
        }
    }

    /**
     * 上屏一个候选词，结束这次组合。
     *
     * `thenSuggest = false` 只有一个调用点：标点键在粘标点之前的那次上屏。
     * 那时如果还去查联想，`→` 一排会紧跟在一个逗号后面冒出来，纯噪音。
     */
    private fun commitWord(word: String, thenSuggest: Boolean = true) {
        val ic = currentInputConnection ?: return
        // commitText 会替换掉正在组合的区域，不需要先 finishComposingText；
        // 先 finish 再 commit 在部分宿主里会多出一次光标移动。
        ic.commitText(word, 1)

        val prev = prevWord
        prevWord = if (thenSuggest) word else ""
        if (thenSuggest) {
            // 「上一词 → 这一词」才是学习对象。第一次上屏时 prev 为空，
            // 没有上下文可学，`record` 自己也会拒掉，这里提前跳过省一次 JNI。
            NativeEngine.record(prev, word)
        }

        pending.setLength(0)
        candidates = emptyList()
        suggestions = if (thenSuggest && chinese) {
            NativeEngine.suggest(word, SUGGEST_LIMIT)
        } else {
            emptyList()
        }
        renderCandidates()
        refreshStatus()
    }

    /** 点了联想词：上屏，并把联想链接着往下续（「北京 → 时间 → 安排」）。 */
    private fun commitSuggestion(word: String) {
        val ic = currentInputConnection ?: return
        ic.commitText(word, 1)

        val prev = prevWord
        prevWord = word
        NativeEngine.record(prev, word)

        pending.setLength(0)
        candidates = emptyList()
        suggestions = NativeEngine.suggest(word, SUGGEST_LIMIT)
        renderCandidates()
        refreshStatus()
    }

    private fun commitPending() {
        if (pending.isEmpty()) return
        val ic = currentInputConnection ?: return
        ic.commitText(pending.toString(), 1)
        // 原样上屏的是一串字母（或英文），不是中文词，不能拿它当联想上下文
        prevWord = ""
        clearComposing()
    }

    private fun clearComposing() {
        pending.setLength(0)
        candidates = emptyList()
        suggestions = emptyList()
        renderCandidates()
        refreshStatus()
    }

    /** 上下文断了（标点 / 空格 / 回车 / 删字 / 切模式）：联想和它的依据一起清掉。 */
    private fun forgetContext() {
        prevWord = ""
        if (suggestions.isEmpty()) return
        suggestions = emptyList()
        renderCandidates()
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
                // 双拼码是屏幕上第一处真正来自 Rust 引擎的东西：打 `nihao` 时
                // 这里出现 `nihc`，说明 .so 加载成功、符号对得上、字符串能来回。
                val code = NativeEngine.encodePhrase(pending.toString())
                if (code.isNotEmpty()) append("→").append(code)
            }
            if (!NativeEngine.available) {
                append("  ·  引擎未加载")
            }
        }
    }

    override fun onFinishInput() {
        prevWord = ""
        clearComposing()
        super.onFinishInput()
    }

    private fun dp(v: Float): Int = (v * resources.displayMetrics.density + 0.5f).toInt()

    companion object {
        private const val TAG = "FeiheIme"

        /**
         * 联想最多给几个。
         *
         * 4 是「一排能扫完」和「有点可挑」的折中：候选条是横向滚动的，联想排在
         * 候选后面，给多了用户得先滑一下才看得见，那就等于没有。
         */
        private const val SUGGEST_LIMIT = 4
    }
}
