package com.feihe.ime

import android.content.Intent
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
import com.feihe.ime.keyboard.KeyboardPages
import com.feihe.ime.keyboard.Page

/**
 * 飞鹤输入法 · Android 壳（M2 阶段）。
 *
 * 现在的状态：**26 键 + 双拼/全拼双模 + 真候选**。
 *
 * 按键串怎么解释由设置里的「输入方式」决定（`config.ini` 的 `pinyin_mode`，
 * 和 Windows 端同一个字段）：
 *
 * - **双拼（默认）**：`nihc` 原样当码查，出「你好」。查不到时**自动按全拼再试一次**，
 *   所以直接打 `nihao` 也出「你好」——手机上第一次装完不会一片空白。
 * - **全拼**：一律按全拼解释。`xian` 出「先」，不是「西安」。适合不熟悉双拼的人。
 *
 * ## 这一版刻意没做的（别以为是漏了）
 *
 * - **分词态**（打一长串码 `nihcshij` 再切段逐段选）：Windows 端有，手机上
 *   交互太重（要左右移段）。现在长串只在「恰好是某个词的前缀」时有候选。
 * - **数字选词**：键盘没有数字行，加一行会挤掉字母键的高度。选词靠点。
 * - **联想浮窗**：有联想，但**不弹独立窗口**（那会压住键盘、还会把候选条顶跳）。
 *   联想词接在候选条末尾，用 `→` 前缀区分，点一下上屏。
 * - **上档锁定**：⇧ 是一次性的（打一个大写就落下），没有双击锁定大小写。
 */
class FeiheIme : InputMethodService() {

    /** 正在组合的字母（还没上屏）。双拼模式下它就是码，全拼模式下还要翻译一次。 */
    private val pending = StringBuilder()

    /** 当前候选。空表示没有候选（码还没成型，或者词库里没有）。 */
    private var candidates: List<String> = emptyList()

    /**
     * 这一串按键**实际用到的码**（引擎算出来的）。
     *
     * 全拼模式下 `nihao` 会变成 `nihc`，状态条要显示这个箭头——它是引擎真的在干活的
     * 唯一可见证据；双拼模式下两者相同，就不显示。空串 = 这一串还成不了码。
     */
    private var code: String = ""

    /**
     * 输入方式：true = 全拼，false = 双拼（默认）。
     *
     * 缓存起来是因为它每次按键都要用，而 `config` 那边要过锁。设置页改完之后
     * `onStartInputView` / `onWindowShown` 会重读一次——同一个进程，改完立刻生效。
     */
    private var pinyinMode: Boolean = false


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

    /**
     * 上档（一次性）。
     *
     * **一次性是刻意的**：桌面 Shift 按住不放才是大写，手机上没有「按住」，双击锁
     * 定又容易误触。所以按一下 ⇧ = 下一个字母大写，打完自动落下来。要连打大写，
     * 现在的办法是连点——知道不爽，见 `docs/后续待办.md` 的「上档锁定」。
     */
    private var shifted = false

    private var statusText: TextView? = null
    private var statusBar: LinearLayout? = null
    private var candidateBar: LinearLayout? = null
    private var candidateScroll: HorizontalScrollView? = null

    /** 键盘容器。字母页和数字页在这里换，**不是**重建整个 InputView。 */
    private var keyboardHost: LinearLayout? = null

    /** 当前键盘页。字母 / 数字 / 符号——布尔值装不下三态，别再退回布尔。 */
    private var page: Page = Page.LETTERS

    override fun onCreate() {
        super.onCreate()
        // 数据目录用 filesDir：输入法进程随时可能被杀，配置和词频都必须落盘到
        // 应用私有目录（不是缓存目录——缓存会被系统清掉）。
        NativeEngine.init(filesDir.absolutePath)
        readInputMode()
        Log.i(TAG, "onCreate: 引擎 ${NativeEngine.coreVersion()}，${modeLabel()}")
    }

    /**
     * 从配置里重读输入方式。
     *
     * 设置页和输入法跑在**同一个进程**里，所以那边 `setConfig` 改完，这边的
     * `config::get()` 立刻能读到新值——不需要重载文件。重读的时机是「键盘再次
     * 出现」（`onStartInputView` / `onWindowShown`），用户从设置页返回就能用上。
     */
    private fun readInputMode() {
        pinyinMode = NativeEngine.settings()?.pinyin ?: pinyinMode
    }

    private fun modeLabel(): String = if (pinyinMode) "全拼" else "双拼"


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
        // 用户可能刚从设置页切了输入方式/方案回来，这里重读一次
        readInputMode()
        refreshStatus()
    }

    /**
     * 键盘窗口每次露出来都重读一次输入方式。
     *
     * `onStartInputView` 不一定每次都来（同一个输入框里收起再弹出可能只走这里），
     * 而设置页返回后最需要的就是「改完立刻生效」。读一次配置只是一个短串的 JNI，
     * 比让用户怀疑「设置没生效」便宜得多。
     */
    override fun onWindowShown() {
        super.onWindowShown()
        readInputMode()
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
        }

        // 状态条右侧的齿轮。**输入法必须有这个入口**：系统设置里的那一层藏得深，
        // 用户第一次装完根本不知道去哪调方案。
        val gear = TextView(this).apply {
            text = "⚙"
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 18f)
            gravity = Gravity.CENTER
            setTextColor(0xFF5F6368.toInt())
            setPadding(dp(14f), dp(6f), dp(14f), dp(6f))
            contentDescription = "输入法设置"
            setOnClickListener { openSettings() }
        }

        statusBar = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setBackgroundColor(0xFFEDEEF0.toInt())
            addView(
                statusText,
                LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f),
            )
            addView(gear)
        }
        root.addView(
            statusBar,
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

        keyboardHost = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
        }
        root.addView(keyboardHost)
        showPage(Page.LETTERS)

        refreshStatus()
        refreshCandidates()
        return root
    }

    /**
     * 换键盘页。三页共用同一个容器，**不重建 InputView**——
     * 重建会走一遍 `onCreateInputView`，键盘会闪一下、候选条也会清空。
     *
     * 行高按页给（字母页 46dp×4，紧凑页 36dp×5），总高度几乎一致，切页时
     * 输入框和候选条不会往上跳。理由和数字页的键位一样在 `KeyboardLayout.kt`。
     */
    private fun showPage(target: Page) {
        val host = keyboardHost ?: return
        page = target
        host.removeAllViews()
        host.addView(
            KeyboardView.build(
                this,
                KeyboardPages.rows(target),
                shifted,
                KeyboardPages.rowHeightDp(target),
            ) { onKey(it) },
        )
        refreshStatus()
    }

    private fun openSettings() {
        // 输入法进程里启动 Activity 必须带 NEW_TASK：这个 Context 不是一个 Activity。
        val i = Intent(this, SettingsActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        runCatching { startActivity(i) }
    }

    private fun onKey(key: Key) {
        when (key.kind) {
            KeyKind.CHAR -> {
                // 一开始打新东西，旧联想就必须消失：留着它等于把「几分钟前的上下文」
                // 又摆在手指下面，点错了上屏的词完全不搭。
                suggestions = emptyList()
                if (shifted) {
                    shifted = false
                    KeyboardView.applyShiftState(keyboardHost, false)
                    if (chinese) {
                        // 中文态 + 上档 = **临时英文**：先把手上半截拼音处理掉再上屏，
                        // 否则「nihao」+ ⇧A 会变成组合态里的「nihaoA」。
                        // 上屏的这个大写字母也不参与联想（上下文已经断了）。
                        if (candidates.isNotEmpty()) {
                            commitWord(candidates.first(), thenSuggest = false)
                        } else {
                            commitPending()
                        }
                        commitText(key.code.uppercase())
                        forgetContext()
                        return
                    }
                    // 英文态：正常续进组合串，H e l l o 这样一个个打
                    pending.append(key.code.uppercase())
                    pushComposing()
                    return
                }
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
                shifted = false
                KeyboardView.applyShiftState(keyboardHost, false)
                // 切英文时把中文候选丢掉：那批候选是按中文码查的，留着只会误导
                forgetContext()
                refreshCandidates()
            }

            KeyKind.PAGE -> showPage(
                when (key.code) {
                    "letters" -> Page.LETTERS
                    "symbols" -> Page.SYMBOLS
                    else -> Page.DIGITS
                },
            )

            KeyKind.TEXT -> {
                // 和标点同一套：先把手上半截拼音处理掉，再打这个字符。
                // 不这么做的话「nihao」+「1」会变成「nihao1」还在组合态里。
                if (chinese && candidates.isNotEmpty()) {
                    commitWord(candidates.first(), thenSuggest = false)
                } else {
                    commitPending()
                }
                commitText(key.code)
                forgetContext()
            }

            KeyKind.SHIFT -> {
                shifted = !shifted
                // 只改这一颗键的配色，不重建键盘（重建会闪、候选条会抖）
                KeyboardView.applyShiftState(keyboardHost, shifted)
            }
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
     * 中文态下把已输入的字母交给引擎，引擎决定「当双拼码还是当全拼」，并把实际用的
     * 码一起返回（`code` 字段，给状态条）。**判断不在 Kotlin 里做**——那是第二份
     * 真值，Windows/Android 一旦分歧就只能靠人肉对，而且症状是「候选莫名其妙不对」。
     *
     * 编码失败（还没打成完整音节，比如 `nih`）就没有候选，但**不清空已输入的
     * 字母**——用户还在打。
     */
    private fun refreshCandidates() {
        if (!chinese || pending.isEmpty()) {
            code = ""
            candidates = emptyList()
        } else {
            val (c, words) = NativeEngine.lookupInput(pending.toString(), pinyinMode, 9)
            code = c
            candidates = words
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
        code = ""
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
        code = ""
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
        code = ""
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
            // 切了页就写页名（数字页上写「双拼」会让人以为还能打拼音）。
            // 字母页写「双拼/全拼」而不是「26 键」——键盘长什么样看得见，
            // 输入方式是看不见的，而它决定了「为什么我打 xian 出了西安」。
            append(if (page == Page.LETTERS) " · " + modeLabel() else " · " + KeyboardPages.label(page))
            if (pending.isNotEmpty()) {
                append(" · ")
                append(pending)
                // 双拼模式下码就是输入的字母，两者相同，不显示箭头；
                // 全拼模式（或双拼模式退回全拼解释）才有 `nihao→nihc`。
                if (code.isNotEmpty() && code != pending.toString()) {
                    append("→").append(code)
                }
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
