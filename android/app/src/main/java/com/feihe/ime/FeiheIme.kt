package com.feihe.ime

import android.content.Intent
import android.graphics.drawable.GradientDrawable
import android.inputmethodservice.InputMethodService
import android.util.Log
import android.util.TypedValue
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.widget.Toast
import android.view.inputmethod.EditorInfo
import android.widget.HorizontalScrollView
import android.widget.LinearLayout
import android.widget.TextView
import com.feihe.ime.engine.NativeEngine
import com.feihe.ime.segment.SegmentState
import com.feihe.ime.suggest.CandidateBarPolicy
import com.feihe.ime.suggest.HostContext
import com.feihe.ime.voice.VoiceRecorder
import com.feihe.ime.keyboard.Key
import com.feihe.ime.keyboard.KeyKind
import com.feihe.ime.keyboard.KeyboardView
import com.feihe.ime.keyboard.KeyboardPages
import com.feihe.ime.keyboard.NumberSelect
import com.feihe.ime.keyboard.NumberSelectLayout
import com.feihe.ime.keyboard.Page
import com.feihe.ime.keyboard.ShiftMachine
import com.feihe.ime.keyboard.ShiftState

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
     * 逐段选择态。`null` = 普通候选条。见 A1 和 `SegmentState`。
     *
     * 只有「整码查不到、引擎给了自动整句」时才非 null —— 那时单个候选是一整句，
     * 用户本来没法改其中某一段，正是分段态要解决的。
     */
    private var segmentState: SegmentState? = null

    private val segmentActive: Boolean get() = segmentState != null

    /**
     * 上档三态：关 / 一次性 / 锁定。
     *
     * **单击 ⇧ = 一次性**（打完一个字母自动落下），**长按 ⇧ = 锁定**（连续大写，
     * 再点一下解除）。不用双击：键盘里 ⌫ 长按已是既有手势，双击会和「连按两下打两个
     * 大写」抢时间窗口。状态迁移全在 [ShiftMachine] 里，那边有 JVM 单测盯着。
     */
    private var shiftState = ShiftState.OFF

    /** 「数字选词行」开关。设置页可关；关掉后这行连组合态都不出现。 */
    private var showNumberRow = true

    /** 一屏候选上限（5/7/9）。查引擎和渲染共用同一个数，避免两处不一致。 */
    private var candidateLimit = CandidateBarPolicy.DEFAULT_LIMIT

    private var statusText: TextView? = null
    private var micButton: TextView? = null
    private var statusBar: LinearLayout? = null
    private var voice: VoiceRecorder? = null
    private var voiceEnabled = true
    private var candidateBar: LinearLayout? = null
    private var candidateScroll: HorizontalScrollView? = null

    /**
     * 数字选词行的容器。**和键盘分开**：它是候选条下方一个可以 `GONE` 的独立容器，
     * 没候选时高度为 0，字母键一 dp 不少。见 [NumberSelectLayout]。
     */
    private var numberRowHost: LinearLayout? = null

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
        voice = VoiceRecorder(
            this,
            onState = { s -> onVoiceState(s) },
            onResult = { text ->
                // 识别结果直接上屏。先把组合态收掉，否则会和半截拼音拼在一起。
                clearComposing()
                commitText(text)
                forgetContext()
            },
        )
        readUiPrefs()
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

    /**
     * 界面开关单独放 SharedPreferences，**不进 core 的 config.ini**。
     *
     * `config.ini` 是 Windows / Android 共享的引擎配置（输入方式、词频、联想……），
     * 而「要不要显示数字选词行」纯粹是手机上的一排按钮，桌面端没有对应概念，
     * 塞进去只会让两边各多一个永远用不到的字段。见 `UiPrefs`。
     */
    private fun readUiPrefs() {
        showNumberRow = UiPrefs.showNumberRow(this)
        voiceEnabled = UiPrefs.voiceEnabled(this)
        candidateLimit = UiPrefs.candidateLimit(this)
        micButton?.visibility = if (voiceEnabled) View.VISIBLE else View.GONE
    }

    private fun modeLabel(): String = if (pinyinMode) "全拼" else "双拼"


    override fun onStartInput(info: EditorInfo?, restarting: Boolean) {
        super.onStartInput(info, restarting)
        clearComposing()
        // 换了输入框（甚至可能是换了 App），**同一段连续输入里攒的 prevWord 不能再沿用** ——
        // 否则切到微信会拿着半小时前在便签里打的词去联想。
        // 但也不能直接清空：搜狗这类输入法会读光标前的文字，切回聊天框时「北京」后面
        // 还是能接着联想「时间」。所以改成从宿主前文里取上一个词。
        prevWord = readHostContext(info)
        // 顺便把这一词的联想摆出来：切回输入框不用先打一个字才看得到。
        suggestions = if (prevWord.isNotEmpty() && chinese) {
            NativeEngine.suggest(prevWord, SUGGEST_LIMIT)
        } else {
            emptyList()
        }
        renderCandidates()
        Log.i(
            TAG,
            "onStartInput: inputType=0x${(info?.inputType ?: 0).toString(16)} " +
                "restarting=$restarting fieldId=${info?.fieldId} prev=$prevWord"
        )
    }

    /**
     * 读光标前的文字，取「上一个词」当联想上下文。
     *
     * **密码框一个字都不读**（[HostContext.canRead] 里按 inputType 位掩码判），
     * 和 Windows 端「密码框不接管」是同一条底线。取 24 个字符足够覆盖一个词，
     * 又不会把整段聊天记录拉进内存。IC 还没就绪 / 宿主拒绝时静默返回空串。
     */
    private fun readHostContext(info: EditorInfo?): String {
        if (!HostContext.canRead(info?.inputType ?: 0)) return ""
        val before = runCatching {
            currentInputConnection?.getTextBeforeCursor(24, 0)
        }.getOrNull()
        return HostContext.lastWord(before)
    }

    override fun onStartInputView(info: EditorInfo?, restarting: Boolean) {
        super.onStartInputView(info, restarting)
        // 用户可能刚从设置页切了输入方式/方案/界面开关回来，这里重读一次
        readInputMode()
        readUiPrefs()
        refreshCandidates()
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
        readUiPrefs()
        refreshCandidates()
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

        // 麦克风：按住说话、松手识别。**用 OnTouch 而不是 OnClick**：两态按钮用户
        // 分不清现在是录还是没录，而输入法窗口没地方放录音红灯。
        micButton = TextView(this).apply {
            text = "🎤"
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 17f)
            gravity = Gravity.CENTER
            setPadding(dp(12f), dp(6f), dp(12f), dp(6f))
            contentDescription = "按住说话"
            visibility = if (voiceEnabled) View.VISIBLE else View.GONE
            setOnTouchListener { v, ev ->
                when (ev.actionMasked) {
                    MotionEvent.ACTION_DOWN -> {
                        v.isPressed = true
                        voice?.start()
                        true
                    }
                    MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                        v.isPressed = false
                        voice?.stop()
                        true
                    }
                    else -> false
                }
            }
        }

        statusBar = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setBackgroundColor(0xFFEDEEF0.toInt())
            addView(
                statusText,
                LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f),
            )
            addView(micButton)
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

        // 数字选词行：夹在候选条和键盘之间。没候选时整行 GONE（高度 0），
        // 所以它不占字母键的高度——这是 A2「不挤掉字母高度」的落点。
        numberRowHost = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            visibility = View.GONE
        }
        root.addView(
            numberRowHost,
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.WRAP_CONTENT,
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
        // 换页时把上档一起清掉：数字/符号页没有"大写"的概念，留着锁定态
        // 回到字母页会让用户莫名其妙打出一串大写。
        shiftState = ShiftState.OFF
        host.removeAllViews()
        host.addView(
            KeyboardView.build(
                this,
                KeyboardPages.rows(target),
                shiftState,
                KeyboardPages.rowHeightDp(target),
                onLongKey = { onLongKey(it) },
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

    /** 语音状态 → 状态条提示。权限/模型缺失给一句能照做的话，不静默失败。 */
    private fun onVoiceState(s: VoiceRecorder.State) {
        micButton?.text = if (s == VoiceRecorder.State.RECORDING ||
            s == VoiceRecorder.State.RECOGNIZING ||
            s == VoiceRecorder.State.LOADING
        ) "🔴" else "🎤"
        when (s) {
            VoiceRecorder.State.LOADING -> refreshStatus("正在加载语音模型…")
            VoiceRecorder.State.RECORDING -> refreshStatus("正在录音，松手上屏")
            VoiceRecorder.State.RECOGNIZING -> refreshStatus("正在识别…")
            VoiceRecorder.State.NO_PERMISSION -> {
                refreshStatus("需要麦克风权限，请在设置里开启语音输入")
                Toast.makeText(this, "请先授权麦克风权限", Toast.LENGTH_SHORT).show()
            }
            VoiceRecorder.State.NO_MODEL -> refreshStatus("语音模型缺失")
            VoiceRecorder.State.FAILED -> refreshStatus("语音识别失败")
            VoiceRecorder.State.IDLE -> refreshStatus()
        }
    }

    private fun onKey(key: Key) {
        when (key.kind) {
            KeyKind.CHAR -> {
                // **联想在打字时保留**（原来一敲字母就清掉）。那个老决定的理由是
                // "留着旧联想等于把几分钟前的上下文摆在手指下面"，但现在两处已经变了：
                // ① onStartInput 会读宿主前文重设 prevWord，切窗口后不再是陈旧的上下文；
                // ② 每次 commitWord 都会按刚上屏的词重算 suggestions。
                // 保留它才能让候选条末尾的 →词 真正点得到，也和搜狗一致。
                // 代价是候选和联想同屏，所以 refreshCandidates 里按
                // CandidateBarPolicy 把候选压到 6 个，给联想留出位置。
                if (ShiftMachine.upper(shiftState)) {
                    // 一次性打完就落下；锁定态保持。状态迁移在 ShiftMachine 里（有单测）。
                    shiftState = ShiftMachine.afterChar(shiftState)
                    KeyboardView.applyShiftState(keyboardHost, shiftState)
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
                // 展开某段时，退格先收起该段（和「返回」一个意思），别直接删拼音。
                // chip 视图下则照常删一个字母——用户还在编辑那串码。
                val expanded = segmentState
                if (expanded != null && expanded.focus >= 0) {
                    segmentState = expanded.back()
                    renderCandidates()
                } else if (pending.isNotEmpty()) {
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
                // 分段态：空格 = 整句上屏（chip 视图末尾写了「↵上屏」提示）。
                // 必须放在最前面，否则会落到「选第一个候选」把整句当成第一条候选。
                if (commitSegmentIfActive()) return
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
                if (!commitSegmentIfActive()) {
                    if (chinese && candidates.isNotEmpty()) {
                        commitWord(candidates.first(), thenSuggest = false)
                    } else {
                        commitPending()
                    }
                }
                commitText(key.code)
                forgetContext()
            }

            KeyKind.ENTER -> {
                if (!commitSegmentIfActive()) commitPending()
                commitText("\n")
                forgetContext()
            }

            KeyKind.MODE -> {
                chinese = !chinese
                shiftState = ShiftState.OFF
                KeyboardView.applyShiftState(keyboardHost, shiftState)
                // 切英文时把中文候选丢掉：那批候选是按中文码查的，留着只会误导
                forgetContext()
                refreshCandidates()
            }

            KeyKind.SELECT -> {
                // 分段态下数字行是隐藏的，这里再挡一道：万一残影被点到，
                // 也不能拿"第 N 个候选"去解释一个分段 chip 的下标。
                if (segmentActive) return
                // 数字选词：code 是 "1".."9"，越界（候选没那么多）直接忽略。
                // 下标换算在 NumberSelect.indexOf 里，那边钉死了 -1。
                val digit = key.code.toIntOrNull() ?: return
                val index = NumberSelect.indexOf(digit, candidates.size) ?: return
                commitWord(candidates[index])
            }

            KeyKind.PAGE -> showPage(
                when (key.code) {
                    "letters" -> Page.LETTERS
                    "symbols" -> Page.SYMBOLS
                    else -> Page.DIGITS
                },
            )

            KeyKind.TEXT -> {
                // 和标点同一套：先把手上半截拼音（或分段整句）处理掉，再打这个字符。
                // 不这么做的话「nihao」+「1」会变成「nihao1」还在组合态里。
                if (!commitSegmentIfActive()) {
                    if (chinese && candidates.isNotEmpty()) {
                        commitWord(candidates.first(), thenSuggest = false)
                    } else {
                        commitPending()
                    }
                }
                commitText(key.code)
                forgetContext()
            }

            KeyKind.SHIFT -> {
                shiftState = ShiftMachine.onTap(shiftState)
                // 只改这一颗键的配色/标签，不重建键盘（重建会闪、候选条会抖）
                KeyboardView.applyShiftState(keyboardHost, shiftState)
            }
        }
    }

    /** 长按回调。目前只有 ⇧ 用：切锁定态。⌫ 的连删在 KeyboardView 内部消化了。 */
    private fun onLongKey(key: Key) {
        if (key.kind != KeyKind.SHIFT) return
        shiftState = ShiftMachine.onLongPress(shiftState)
        KeyboardView.applyShiftState(keyboardHost, shiftState)
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
            // 有联想时候选少给几个，让候选条末尾的 →词 不用横滑就看得见。
            // 上限来自设置页；算式在 CandidateBarPolicy（有 JVM 单测）。
            val slots = CandidateBarPolicy.candidateSlots(candidateLimit, suggestions.isNotEmpty())
            val (c, words) = NativeEngine.lookupInput(pending.toString(), pinyinMode, slots)
            code = c
            candidates = words
        }
        refreshSegmentMode()
        renderCandidates()
    }

    /**
     * 要不要进「逐段选择」态。
     *
     * 判据**完全来自引擎**：`segmentCandidates` 返回空就是普通候选。这里加的
     * `candidates.size == 1 && code.length >= 4` 只是省一次 JNI 的短路条件（整句
     * 按定义只有一条候选），不是第二份判断规则——真值仍在 `lookup_input.segments`。
     */
    private fun refreshSegmentMode() {
        segmentState = if (chinese && candidates.size == 1 && code.length >= 4) {
            val segs = NativeEngine.segmentCandidates(pending.toString(), pinyinMode, 9)
            if (segs.isEmpty()) {
                null
            } else {
                SegmentState(segs.map { it.words }).takeIf { it.active }
            }
        } else {
            null
        }
    }

    /** 分段态下按了收尾键（空格/回车/标点/字符）：先把整句上屏。 */
    private fun commitSegmentIfActive(): Boolean {
        val state = segmentState ?: return false
        commitWord(state.sentence(), thenSuggest = false)
        return true
    }

    private fun renderCandidates() {
        val bar = candidateBar ?: return
        bar.removeAllViews()
        renderNumberRow()
        val seg = segmentState
        if (seg != null) {
            renderSegmentBar(bar, seg)
            return
        }
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
     * 画分段 chip（A1）。
     *
     * 两种视图：
     * - **chip 视图**：一排 `[张三][李]`，点哪段就展开哪段。已展开过的段用蓝字标出
     *   （用户改过的地方得看得出来）。末尾一个「↵上屏」提示，说明空格能整句上屏。
     * - **展开视图**：`‹返回` + 该段候选，点一个就换掉并回到 chip 视图。
     *
     * 没有左右移段、没有游标——手机上单行候选条放不下那套交互（见 SegmentState）。
     */
    private fun renderSegmentBar(bar: LinearLayout, state: SegmentState) {
        if (state.focus >= 0) {
            bar.addView(
                TextView(this).apply {
                    text = "‹ 返回"
                    setTextSize(TypedValue.COMPLEX_UNIT_SP, 16f)
                    setTextColor(0xFF5F6368.toInt())
                    setPadding(dp(10f), 0, dp(14f), 0)
                    setOnClickListener {
                        segmentState = state.back()
                        renderCandidates()
                    }
                },
            )
            for ((i, word) in state.focusedWords().withIndex()) {
                bar.addView(
                    TextView(this).apply {
                        text = word
                        setTextSize(TypedValue.COMPLEX_UNIT_SP, 20f)
                        setPadding(dp(12f), 0, dp(12f), 0)
                        setTextColor(
                            if (i == state.selected[state.focus]) 0xFF1A6FD4.toInt()
                            else 0xFF202124.toInt()
                        )
                        setOnClickListener {
                            segmentState = state.choose(i)
                            renderCandidates()
                        }
                    },
                )
            }
            return
        }

        for (i in state.segments.indices) {
            val changed = state.selected[i] != 0
            bar.addView(
                TextView(this).apply {
                    text = state.wordAt(i)
                    setTextSize(TypedValue.COMPLEX_UNIT_SP, 20f)
                    setPadding(dp(10f), dp(4f), dp(10f), dp(4f))
                    setTextColor(if (changed) 0xFF1A6FD4.toInt() else 0xFF202124.toInt())
                    // 分段之间画一道竖线，否则「张三李」三个 chip 会看成一个词
                    background = GradientDrawable().apply {
                        setStroke(dp(1f), 0xFFD9DCE1.toInt())
                        cornerRadius = dp(4f).toFloat()
                    }
                    setOnClickListener {
                        segmentState = state.open(i)
                        renderCandidates()
                    }
                },
            )
        }
        bar.addView(
            TextView(this).apply {
                text = "  ↵上屏"
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 13f)
                setTextColor(0xFF9AA0A8.toInt())
                setPadding(dp(10f), 0, dp(10f), 0)
            },
        )
    }

    /**
     * 画数字选词行。
     *
     * 候选变化、上屏、清组合都会走到 `renderCandidates()`，所以这一行跟着一起刷新，
     * 不需要额外的失效逻辑。**没候选 / 设置里关了 / 正在逐段选择** 时整行 `GONE`，
     * 高度归零，字母键一 dp 不少。
     */
    private fun renderNumberRow() {
        val host = numberRowHost ?: return
        host.removeAllViews()
        val rows = if (showNumberRow && !segmentActive) {
            NumberSelectLayout.rows(candidates.size)
        } else {
            emptyList()
        }
        if (rows.isEmpty()) {
            host.visibility = View.GONE
            return
        }
        host.visibility = View.VISIBLE
        host.addView(
            KeyboardView.build(
                this,
                rows,
                ShiftState.OFF,
                NumberSelectLayout.ROW_HEIGHT_DP,
            ) { onKey(it) },
        )
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
        // 整句上屏后组合结束，分段态必须一起收掉，否则候选条会留着一排旧 chip
        segmentState = null
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
        segmentState = null
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
        segmentState = null
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

    /** 覆盖状态条文案（语音过程用）。传 null / 不传就恢复常规状态。 */
    private fun refreshStatus(override: String? = null) {
        if (override != null) {
            statusText?.text = override
            return
        }
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
            // 分段态给一句提示，否则用户不知道候选条上一排方块是什么、怎么退出。
            val seg = segmentState
            if (seg != null) {
                append(if (seg.focus >= 0) "  ·  选这一段的字" else "  ·  点词改某一段")
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
         * 5 和 core 的 `next::SHOW`（桌面端候选窗）对齐：两边同一份词对表，
         * 给出条数不一样会让人以为「手机上联想更少」。刚上屏、没有候选时这 5 个
         * 正好排满一行；正在打字时它们排在候选后面，多出来的部分横向滑一下。
         */
        private const val SUGGEST_LIMIT = 5
    }
}
