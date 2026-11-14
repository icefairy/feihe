package com.feihe.ime

import android.app.AlertDialog
import android.content.ClipboardManager
import android.content.Intent
import android.database.ContentObserver
import android.net.Uri
import android.graphics.drawable.GradientDrawable
import android.inputmethodservice.InputMethodService
import android.os.Handler
import android.os.Looper
import android.text.TextUtils
import android.util.Log
import android.util.TypedValue
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.widget.FrameLayout
import android.widget.Toast
import android.view.inputmethod.EditorInfo
import android.view.KeyEvent
import android.view.inputmethod.ExtractedTextRequest
import android.view.inputmethod.InputConnection
import android.widget.HorizontalScrollView
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import com.feihe.ime.clip.ClipHistory
import com.feihe.ime.clip.OtpCode
import com.feihe.ime.clip.SmsOtp
import com.feihe.ime.engine.NativeEngine
import com.feihe.ime.segment.SegmentState
import com.feihe.ime.suggest.EmailSuggest
import com.feihe.ime.suggest.HostContext
import com.feihe.ime.voice.VoiceEngine
import com.feihe.ime.voice.VoiceRecorder
import com.feihe.ime.keyboard.Backspace
import com.feihe.ime.keyboard.CandidateGrid
import com.feihe.ime.keyboard.Key
import com.feihe.ime.keyboard.KeyKind
import com.feihe.ime.keyboard.EnterSpec
import com.feihe.ime.keyboard.KeyboardView
import com.feihe.ime.keyboard.KeyboardPages
import com.feihe.ime.keyboard.NavAction
import com.feihe.ime.keyboard.NavCursor
import com.feihe.ime.keyboard.BackspaceSplit
import com.feihe.ime.keyboard.NumberSelect
import com.feihe.ime.keyboard.NumberSelectLayout
import com.feihe.ime.keyboard.Page
import com.feihe.ime.keyboard.ShiftMachine
import com.feihe.ime.keyboard.ShiftState
import com.feihe.ime.keyboard.AssistExit
import java.io.File

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
     * 邮箱后缀联想（`@qq.com` 这类）。
     *
     * 和 [suggestions] 分开存：那个是词库按上一个词给的**中文词**，这个是按光标前的
     * **纯数字**给的邮箱后缀，来源和触发条件完全不同。混在一起的话，清词库联想时
     * 会连邮箱候选一起清掉，反之亦然。
     */
    private var emailSuffixes: List<String> = emptyList()

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

    /**
     * 这次查询引擎带回的分段信息（`NativeEngine.Lookup.segments`）。
     *
     * 和 [segmentState] 分开存：segmentState 是「进没进分段态」的 UI 状态，
     * 这里是引擎算好的真值。core 在 lookup 里顺手算好搭车回来，壳不再单独调 JNI。
     */
    private var pendingSegments: List<NativeEngine.SegmentChoice> = emptyList()

    /**
     * 快捷输入态：这一串是 `/rq` 这种命令码，不是拼音。
     *
     * 为什么要有独立的状态位，而不是靠「`pending` 以 `/` 开头」现判：引导符来自
     * 引擎（[NativeEngine.quickPrefix]），万一它哪天变了，靠硬编码 `/` 的判断会
     * 静默失效；而且快捷输入的候选**不参与联想、不进词库学习**，语义和拼音完全
     * 不同，混在一处改起来就是一串 if。
     *
     * 只在「中文态 + 组合串为空」时按 `/` 进入；退出方式见 DELETE/SPACE/PUNCT 分支。
     */
    private var quickMode = false

    /** 快捷输入的候选（`/rq` → 「2024-06-01」）。点一个就上屏，空格选第一个。 */
    private var quickWords: List<String> = emptyList()

    private val segmentActive: Boolean get() = segmentState != null

    /**
     * 上档三态：关 / 一次性 / 锁定。
     *
     * **单击 ⇧ = 一次性**（打完一个字母自动落下），**双击 ⇧ = 锁定**（连续大写，
     * 再双击解除）。状态迁移全在 [ShiftMachine] 里，那边有 JVM 单测盯着。
     *
     * 2026-11-12 改：手势从「长按锁定」换成「双击锁定」。理由见 [ShiftMachine.onTapWithTime]。
     */
    private var shiftState = ShiftState.OFF

    /**
     * 上一次点击 ⇧ 的时间戳（毫秒，0 = 没有上一次）。
     *
     * 双击判定需要**跨两次点击**记住时间，所以状态只能放在这里：KeyboardView 是
     * 无状态的 object，键盘页一重建它的成员就被清了，双击永远判不出来。
     * 换页 / 切中英 / 上屏标点这些「上下文断开」的地方要把它归零。
     */
    private var lastShiftTapAt = 0L

    /**
     * 候选条是否展开成多行。
     *
     * 触屏和键鼠的关键差别：电脑要按数字键选词，所以候选一行就够（数字即编号）；
     * 手机上直接用手指点，**一行反而只露出 5~7 个、还要翻页**。展开成多行之后
     * 一屏能看见十几个，点哪个是哪个，不需要任何编号。
     *
     * 展开时行高、候选条总高都会变——输入视图高度变化不会触发宿主 resize（我们
     * 一直在同一个 InputView 里改子 View），见 [renderCandidates]。
     */
    private var candidateExpanded = false

    /** 「数字选词行」开关。设置页可关；关掉后这行连组合态都不出现。 */
    private var showNumberRow = true


    private var statusText: TextView? = null
    private var micButton: TextView? = null
    private var statusBar: LinearLayout? = null
    private var voice: VoiceRecorder? = null

    /** 当前语音状态。麦克风按钮是两态开关，必须知道「现在录没录」才能决定点了做什么。 */
    private var voiceState = VoiceRecorder.State.IDLE

    /**
     * 上一次语音上屏如果被纠错过，这里留着还原所需的信息。
     *
     * 只保留**一次**：撤销是「刚说错的那个词」，用户一旦又敲了别的键，
     * 上下文就变了，再撤销会把不该动的文字改掉。`null` = 没有可撤销的。
     */
    private var voiceUndo: com.feihe.ime.voice.VoiceCorrect.Result? = null

    /** 录音波形，显示在候选条位置。空闲时应为 GONE。 */
    private var voiceWave: com.feihe.ime.voice.VoiceWaveView? = null

    /** 英文单词建议（系统拼写检查器给的）。只在英文组合态非空。 */
    private var englishWords: List<String> = emptyList()

    /** 英文建议查询器。懒建：只有真用到英文输入才连系统服务。 */
    private var englishSuggest: com.feihe.ime.engine.EnglishSuggest? = null

    /**
     * 语音回调跨线程切主线程用。
     *
     * 用 Handler(Looper.getMainLooper()) 而不是 View.post：`InputMethodService`
     * 不是 View，没有 post 方法；而 InputMethodService 本身在自己线程上有 Handler，
     * 但这里要的是「主线程」这个确定的语义。
     */
    private val voiceUiHandler = android.os.Handler(android.os.Looper.getMainLooper())
    private var voiceEnabled = true
    private var candidateBar: LinearLayout? = null
    private var candidateScroll: HorizontalScrollView? = null

    /**
     * 候选条的行容器（纵向）。收起时里面只有 [candidateBar] 一行，展开时有多行。
     *
     * 不用一个 LinearLayout 改 orientation：改了之后 weight 的方向语义跟着变，
     * 复用池里的子 View 会错位。
     */
    private var candidateColumn: LinearLayout? = null

    /** 顶部那一行（状态栏 / 候选条共用），固定高度见 [TOP_SLOT_DP]。 */
    private var topSlot: FrameLayout? = null

    /** 展开态翻到第几页（0 起）。收起态按屏宽分页，见 [visibleCandidates]。 */
    private var candPage = 0

    /**
     * 展开态已经渲染了多少个候选（懒加载的游标）。
     *
     * 展开时先给 [CandidateGrid.EXPAND_BATCH] 个，滑到底再追加一批。
     * 候选变化（新的一次查询）时必须归零，否则新候选会沿用旧的高度。
     */
    private var expandedLoaded = 0

    /**
     * 数字选词行的容器。**和键盘分开**：它是候选条下方一个可以 `GONE` 的独立容器，
     * 没候选时高度为 0，字母键一 dp 不少。见 [NumberSelectLayout]。
     */
    private var numberRowHost: LinearLayout? = null

    /** 上次画数字行时的内容键（0 = 隐藏）。输入视图重建时复位成 MIN_VALUE 强制重画。 */
    private var numberRowKey = Int.MIN_VALUE

    /** 键盘容器。字母页和数字页在这里换，**不是**重建整个 InputView。 */
    private var keyboardHost: LinearLayout? = null

    /**
     * 展开态专用的「收起」横条。
     *
     * 展开时候选区把键盘整块盖掉了，字母页上那个 ▾ 跟着一起消失——没有这条，
     * 用户展开后就没有任何办法收回去，只能按返回键退出输入法。所以它必须在
     * 展开态常驻在最上面。
     */
    private var collapseBar: LinearLayout? = null

    /**
     * 展开态专用候选容器。
     *
     * 和 [candidateScroll] 二选一显示。为什么不共用一个：HorizontalScrollView
     * 会给子 View 无限宽度约束，展开成多行时 `MATCH_PARENT` 和 `weight` 全部量
     * 不出正确宽度，候选会塌成一团。
     */
    private var expandedColumn: LinearLayout? = null

    /**
     * 包住 [expandedColumn] 的纵向滚动容器。
     *
     * 所有对「展开区可见性」的操作都该走这里，不能直接改 expandedColumn——
     * 它现在只是 ScrollView 里的内容，自己 GONE 只会让 ScrollView 留一个空框。
     */
    private var expandedScroll: ScrollView? = null

    /** 当前键盘页。字母 / 数字 / 符号——布尔值装不下三态，别再退回布尔。 */
    private var page: Page = Page.LETTERS

    /**
     * [segmentState] 是在哪一串按键码上建立的。
     *
     * 用来区分「同一串码重绘」和「码真的变了」——前者必须保住已选的字，
     * 后者才该丢弃。见 [refreshSegmentMode]。
     */
    private var segmentStateCode: String = ""

    /** 当前输入框的 inputType / hint。判断「像不像验证码框」用，见 [OtpCode]。 */
    private var fieldType = 0
    private var fieldHint: CharSequence? = null

    /**
     * 当前输入框的 IME_ACTION（`EditorInfo.imeOptions and IME_MASK_ACTION`）。
     *
     * 回车键要靠它决定「这一下是什么意思」：地址栏是 GO、搜索框是 SEARCH、
     * 聊天框是 NONE（= 换行）。以前完全没存这个值，回车一律当换行——
     * 在浏览器地址栏里就是「不跳转」。
     */
    private var fieldAction = EditorInfo.IME_ACTION_NONE

    /** 候选条上要提示的验证码；null = 没有。只在没有候选时显示，避免顶掉候选的位置。 */
    private var otpCode: String? = null

    /** 辅助码方案：off / ziranma / xiaohe。和 config.ini 的 assist_mode 同一个字段。 */
    private var assistMode = "off"

    /** 形码态是否打开（候选条上的「形」点一下切换）。 */
    private var assistActive = false

    /** 已经打进去的形码字母。打满（自然码 1 / 小鹤 2）之后多按的键会被吞掉。 */
    private val assistKeys = StringBuilder()

    /** 形码态下当前**段**的筛选结果。普通组合态不用它，直接改写 [candidates]。 */
    private var assistWords: List<String> = emptyList()

    /** 剪贴板历史。纯逻辑在 [ClipHistory]，这里只管读、存、画。 */
    private var clipEntries: List<ClipHistory.Entry> = emptyList()
    private var clipPanel: LinearLayout? = null
    private var clipList: LinearLayout? = null
    private var clipButton: TextView? = null
    private var clipOpen = false

    /**
     * 状态条上的「表情」按钮。
     *
     * 2026-11-14 用户要求：把表情入口从字母页底排搬到状态条（和齿轮/剪贴板/
     * 麦克风同一排），这样**任何时候**（字母页/数字页/符号页）都能一键进表情页，
     * 而且不占键盘底排的位置（腾出来的宽度给空格）。
     */
    private var emojiButton: TextView? = null

    /** 状态条上的「收起键盘」按钮。 */
    private var hideButton: TextView? = null


    /** 上次读剪贴板的时间（毫秒）。onStartInputView 和 onWindowShown 常常连着来，别读两遍。 */
    private var lastClipReadAt = 0L

    /** 用户是否打开了「从短信读取验证码」。默认 false，见 [UiPrefs.smsOtp]。 */
    private var smsOtpOn = false

    /** 键盘在屏时挂的短信监听；收起来就摘掉，不做常驻监听。 */
    private var smsObserver: ContentObserver? = null

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
                val fixed = correctVoiceText(text)
                commitText(fixed.text)
                // 被纠错过就挂一个撤销入口：这是机器改的，用户没说「要改」，
                // 必须给一条退路。撤销状态在下一次输入时清掉。
                voiceUndo = fixed.takeIf { it.changed }
                forgetContext()
                if (fixed.changed) {
                    Toast.makeText(
                        this,
                        "已按自造词纠正：${fixed.replaced.joinToString(" ") { "${it.first}→${it.second}" }}",
                        Toast.LENGTH_SHORT,
                    ).show()
                }
            },
            // 音量回调在录音线程，必须切主线程再动 View
            onLevel = { lv ->
                voiceUiHandler.post { voiceWave?.push(lv) }
            },
        )
        // 英文建议器：懒连接系统拼写检查服务，第一次用英文输入时才建
        englishSuggest = com.feihe.ime.engine.EnglishSuggest(this).apply {
            onResult = { prefix, words -> onEnglishSuggest(prefix, words) }
        }
        readUiPrefs()
        clipEntries = ClipHistory.decode(
            runCatching { clipFile().takeIf { it.isFile }?.readText().orEmpty() }.getOrDefault("")
        )
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
        val s = NativeEngine.settings()
        pinyinMode = s?.pinyin ?: pinyinMode
        // 辅助码方案在设置页改完，回到键盘要立刻生效——和输入方式同一个时机重读。
        assistMode = s?.assistMode ?: assistMode
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
        // 数字行开关只改高度，不销毁视图：高度恒定是「不触发宿主 resize」的前提
        numberRowHost?.layoutParams = numberRowHost?.layoutParams?.apply {
            height = if (showNumberRow) dp(NUM_ROW_DP) else 0
        }
        smsOtpOn = UiPrefs.smsOtp(this)
        // 用户刚在设置页把短信开关关掉：监听要立刻摘掉，不能等键盘收起
        if (!smsOtpOn) stopSmsObserver()
        micButton?.visibility = if (voiceEnabled) View.VISIBLE else View.GONE
    }

    private fun modeLabel(): String = if (pinyinMode) "全拼" else "双拼"


    override fun onStartInput(info: EditorInfo?, restarting: Boolean) {
        super.onStartInput(info, restarting)
        clearComposing()
        fieldType = info?.inputType ?: 0
        fieldHint = info?.hintText
        fieldAction = info?.imeOptions?.and(EditorInfo.IME_MASK_ACTION) ?: EditorInfo.IME_ACTION_NONE
        // 换输入框是抓剪贴板的时机：用户刚从短信里复制验证码，切回来正好被记下。
        // 读不到的场合也正常（密码框、宿主不允许），见 readClipboard。
        readClipboard()
        refreshSmsCode(render = false)
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

    /**
     * 读一次剪贴板：记进历史，顺手判断能不能当验证码提示。
     *
     * **只在键盘出现 / 切输入框时读**，不做常驻监听——Android 10 起后台读剪贴板
     * 这条路本来就是断的，输入法能拿到的就是「自己有焦点」的那几个瞬间。用户从短信
     * 复制完切回输入框，正好落在这些时刻上。
     *
     * **密码框一律不读**：剪贴板里很可能就是那个密码，和 [HostContext.canRead]
     * 是同一条底线。
     */
    private fun readClipboard(force: Boolean = false) {
        // onStartInputView 和 onWindowShown 常常连着触发，别读两遍。
        val now = System.currentTimeMillis()
        if (!force && now - lastClipReadAt < 300) return
        lastClipReadAt = now

        if (!HostContext.canRead(fieldType)) {
            otpCode = null
            // 密码框连「上一次的剪贴板」都不该留在引擎里：/fc 会拿它反查
            NativeEngine.setClipboard("")
            return
        }
        val cm = getSystemService(ClipboardManager::class.java) ?: return
        // hasPrimaryClip 先问一遍：部分机型（尤其中低端定制 ROM）在没有主剪贴时
        // primaryClip 直接抛 SecurityException/NPE，被 runCatching 一吞就变成
        // 「点了没反应」，排查时看不出任何线索。
        val has = runCatching { cm.hasPrimaryClip() }.getOrDefault(false)
        if (!has) return
        val clip = runCatching { cm.primaryClip }.getOrNull() ?: return
        val text = runCatching {
            if (clip.itemCount == 0) null else clip.getItemAt(0).coerceToText(this)?.toString()
        }.getOrNull()
        if (text.isNullOrBlank()) return

        // 给 /fc 反查用（core 收不了闭包，只能这样中转）
        NativeEngine.setClipboard(text)
        rememberClip(text)

        // 有数字框 / hint 里的关键词兜底：剪贴板里是验证码，但当前框是普通文本框时
        // 也提示——很多 App 的验证码框不走 number 类型。
        val code = OtpCode.extract(text)
        otpCode = when {
            code == null -> null
            OtpCode.looksLikeOtpField(fieldType, fieldHint) -> code
            OtpCode.hasKeyword(text) -> code
            else -> null
        }
    }

    /**
     * 从短信里取一次验证码。
     *
     * 三道闸全过才会读（见 [SmsOtp]）：设置里打开了、系统给了权限、当前输入框像
     * 验证码框。取到的只是那 4-8 位数字，短信正文用完即弃。
     *
     * [render] 为 false 时只更新状态不动界面——调用方紧接着自己会刷候选，省一次重绘。
     */
    private fun refreshSmsCode(render: Boolean = true) {
        if (!smsOtpOn) return
        if (!OtpCode.looksLikeOtpField(fieldType, fieldHint)) return
        if (!SmsOtp.canRead(this)) return
        val code = SmsOtp.latestCode(this, System.currentTimeMillis())
        if (code == null || code == otpCode) return
        otpCode = code
        if (render) refreshCandidates()
    }

    /**
     * 监听短信变化，只在键盘可见期间挂着。
     *
     * 验证码几乎总是「这个输入框已经开着」的时候到的，所以这里值得挂一个监听：
     * 不挂的话用户得收起键盘再弹出来才能看到提示。收起键盘就摘——不做常驻监听，
     * 也不为了这个去要求后台运行。
     */
    private fun startSmsObserver() {
        if (!smsOtpOn || smsObserver != null) return
        if (!OtpCode.looksLikeOtpField(fieldType, fieldHint)) return
        if (!SmsOtp.canRead(this)) return
        val observer = object : ContentObserver(Handler(Looper.getMainLooper())) {
            override fun onChange(selfChange: Boolean) {
                refreshSmsCode()
            }
        }
        val ok = runCatching {
            contentResolver.registerContentObserver(Uri.parse("content://sms"), true, observer)
        }.isSuccess
        if (ok) smsObserver = observer
    }

    private fun stopSmsObserver() {
        smsObserver?.let { o -> runCatching { contentResolver.unregisterContentObserver(o) } }
        smsObserver = null
    }

    /** 记一条历史。内容没变化时 [ClipHistory.merge] 会返回原引用，这里就不写盘。 */
    private fun rememberClip(text: String) {
        val next = ClipHistory.merge(clipEntries, text, System.currentTimeMillis())
        if (next === clipEntries) return
        clipEntries = next
        persistClips()
        if (clipOpen) renderClipboard()
    }

    private fun clipFile(): File = File(filesDir, "clip.txt")

    private fun persistClips() {
        runCatching { clipFile().writeText(ClipHistory.encode(clipEntries)) }
    }

    private fun toggleClipboard() {
        if (clipOpen) closeClipboard() else openClipboard()
    }

    /**
     * 打开剪贴板面板：藏键盘、露面板。
     *
     * 打开这一刻**强制再读一次**剪贴板：用户点了这个按钮，多半是刚复制了什么，
     * 而系统不允许我们在后台监听剪贴板（Android 10+），这里就是最接近「实时」的时机。
     */
    private fun openClipboard() {
        readClipboard(force = true)
        clipOpen = true
        keyboardHost?.visibility = View.GONE
        clipPanel?.visibility = View.VISIBLE
        clipButton?.setTextColor(Palette.of(this).accent)
        renderClipboard()
    }

    private fun closeClipboard() {
        clipOpen = false
        clipPanel?.visibility = View.GONE
        keyboardHost?.visibility = View.VISIBLE
        clipButton?.setTextColor(Palette.of(this).sub)
    }    private fun renderClipboard() {
        val list = clipList ?: return
        list.removeAllViews()
        val p = Palette.of(this)
        if (clipEntries.isEmpty()) {
            list.addView(
                TextView(this).apply {
                    text = "还没有记录。复制的内容会在键盘弹出时记在这里。"
                    setTextSize(TypedValue.COMPLEX_UNIT_SP, 13f)
                    setTextColor(p.sub)
                    setPadding(dp(10f), dp(14f), dp(10f), dp(14f))
                },
            )
            return
        }
        clipEntries.forEachIndexed { idx, e ->
            if (idx > 0) {
                // 条与条之间的分隔线。没有它，几条连续摆在一起就糊成一片，
                // 分不清是「一条长内容换行了」还是「三条不同的内容」。
                // 用 View 画 1px 细线而不是给 TextView 加 divider：TextView 的
                // background 已经被点击/长按态用掉的可能性更大，画个独立 View 最稳。
                list.addView(
                    View(this).apply { setBackgroundColor(p.separator) },
                    LinearLayout.LayoutParams(
                        LinearLayout.LayoutParams.MATCH_PARENT,
                        (resources.displayMetrics.density * 1f).toInt().coerceAtLeast(1),
                    ),
                )
            }
            list.addView(
                TextView(this).apply {
                    // 预览压成一行：换行会让每一条高度不一样，列表看起来是散开的。
                    // 用户明确要求「复制的内容即使是多行也只显示一行」——多行原文
                    // 照样存着（点上去还是原样上屏），只是预览里收成一行。
                    text = e.text.replace('\n', ' ').replace('\t', ' ').trim()
                    setTextSize(TypedValue.COMPLEX_UNIT_SP, 15f)
                    setTextColor(p.text)
                    maxLines = 1
                    ellipsize = TextUtils.TruncateAt.END
                    setPadding(dp(12f), dp(12f), dp(12f), dp(12f))
                    setOnClickListener { insertClip(e) }
                    setOnLongClickListener {
                        removeClip(e)
                        true
                    }
                },
            )
        }
    }

    /** 点一条 = 上屏。历史里的原文可能有多行，照原样提交。 */
    private fun insertClip(e: ClipHistory.Entry) {
        clearComposing()
        commitText(e.text)
        // 用过的挪到最前，下次打开就是第一条
        clipEntries = ClipHistory.merge(clipEntries, e.text, System.currentTimeMillis())
        persistClips()
        closeClipboard()
    }

    /** 长按删掉这一条。删的是**这一条对象**，不是下标——下标在列表变化后会错位。 */
    private fun removeClip(e: ClipHistory.Entry) {
        val next = clipEntries.filterNot { it === e }
        if (next.size == clipEntries.size) return
        clipEntries = next
        persistClips()
        renderClipboard()
    }

    /**
     * 删光标前一个「字符」，代理对（emoji 这类）一次删干净。
     *
     * 读前文只为拿末尾两个 code unit，所以只请求 2——不能请求 1，那样拿到的
     * 高位代理会被当成完整字符算错。
     *
     * 宿主拒绝读前文时退回删 1：宁可让用户对 emoji 多按一次，也不能拿不准
     * 就删两个把真字吃掉。
     */
    private fun deleteOneCharBackward() {
        val ic = currentInputConnection ?: return
        // **有选区时先删选区**，这是所有主流输入法的行为。
        //
        // `deleteSurroundingText` 只作用于光标位置，对选区**无效**——这正是
        // 「选中一大段之后按退格删不掉」的根因：方法被调用了，宿主收到请求，
        // 但它按「删光标前一个字符」执行，而选区还在，于是什么都没发生。
        // 用户只能按回车（用换行替换选区），于是多出一个空行，还得再删一次。
        val selected = runCatching { ic.getSelectedText(0) }.getOrNull()
        if (!selected.isNullOrEmpty()) {
            ic.commitText("", 1)
            return
        }
        val before = runCatching { ic.getTextBeforeCursor(2, 0) }.getOrNull()
        ic.deleteSurroundingText(Backspace.unitsToDelete(before), 0)
    }

    override fun onStartInputView(info: EditorInfo?, restarting: Boolean) {
        super.onStartInputView(info, restarting)
        // 用户可能刚从设置页切了输入方式/方案/界面开关回来，这里重读一次
        readInputMode()
        readUiPrefs()
        if (info != null) {
            fieldType = info.inputType
            fieldHint = info.hintText
            fieldAction = info.imeOptions and EditorInfo.IME_MASK_ACTION
        }
        // 窗口重新弹出 = 一次新的输入会话，界面状态必须回到初始。
        //
        // 不重置的后果：上次在 A 应用里把候选展开成一屏多行、横滑翻到了第五页，
        // 切到 B 应用时键盘「弹出来就是这样」——用户看到一屏候选、键盘不见了，
        // 完全不知道发生了什么，只能自己找收起箭头。
        resetTransientUiState()
        readClipboard()
        // 短信优先于剪贴板：同一屏里短信里的码一定比剪贴板里的新
        refreshSmsCode(render = false)
        startSmsObserver()
        refreshCandidates()
        refreshStatus()
    }

    /**
     * 把「只属于上一次输入会话」的界面状态清掉。
     *
     * **只重置纯呈现状态**，不动用户偏好（双拼方案、候选个数、样式）——那些是
     * 设置，弹一次键盘就被重置是灾难。
     *
     * 具体三项：
     * - 候选展开 → 收起。展开态会吃满整屏并藏掉键盘，留着等于「键盘不见了」。
     * - 横向滚动 → 归零。横滑位置是上次找某个候选留下的，新会话从第一个看起。
     * - 翻页 → 第一页。
     *
     * `chinese`（中英）**不在这里重置**：它是用户的输入意图，不是呈现状态。
     * 用户在微信里切成英文，切到浏览器仍想打英文——每次弹键盘强制回中文比
     * 「记住上次」更烦人。这一点和搜狗/微信输入法一致。
     */
    private fun resetTransientUiState() {
        candidateExpanded = false
        candPage = 0
        // scrollTo 要在布局完成后才生效：刚弹窗时 candidateColumn 还没量过宽度，
        // 立刻滚动距离是 0。post 到下一帧更稳。
        candidateScroll?.let { sv ->
            sv.scrollTo(0, 0)
            sv.post { sv.scrollTo(0, 0) }
        }
        // 展开容器可能还留着上次的行视图，清掉免得下次展开闪一下旧内容
        expandedColumn?.let {
            it.visibility = View.GONE
            it.removeAllViews()
            (it.layoutParams as? LinearLayout.LayoutParams)?.let { lp ->
                lp.height = dp(CANDIDATE_ROW_DP)
                lp.weight = 0f
                it.layoutParams = lp
            }
        }
    }

    /**
     * 光标或文本变了就重算邮箱联想。
     *
     * 用 `onUpdateSelection` 而不是只在按键时算，是因为三种情况都要覆盖：
     * 用户点进已有文字中间、从别处粘贴一串数字、光标挪到别的输入框。
     * 只在按键里算会漏掉这些，表现就是「有时候联想不出来」。
     *
     * **密码框一律不算**（[HostContext.canRead]），和读前文是同一条底线。
     */
    override fun onUpdateSelection(
        oldSelStart: Int,
        oldSelEnd: Int,
        newSelStart: Int,
        newSelEnd: Int,
        candidatesStart: Int,
        candidatesEnd: Int,
    ) {
        super.onUpdateSelection(
            oldSelStart, oldSelEnd, newSelStart, newSelEnd, candidatesStart, candidatesEnd,
        )
        refreshEmailSuggest()
    }

    /**
     * 读光标前的文字，算出邮箱后缀候选。
     *
     * 只在**中文候选和词库联想都没有**时才占用候选条——用户正在打中文的时候，
     * 候选条该给字词，凭空冒出 `@qq.com` 会挤掉真正要打的字。
     */
    private fun refreshEmailSuggest() {
        val canRead = HostContext.canRead(fieldType)
        val before = if (!canRead) {
            null
        } else {
            runCatching {
                currentInputConnection?.getTextBeforeCursor(EmailSuggest.LOOKBEHIND, 0)
            }.getOrNull()
        }
        val next = EmailSuggest.domainsFor(EmailSuggest.digitsFor(before))
        if (next != emailSuffixes) {
            emailSuffixes = next
            renderCandidates()
        }
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
        readClipboard()
        refreshSmsCode(render = false)
        startSmsObserver()
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
        // 展开列的列数按真实屏宽算（换手机、折叠屏展开、系统字号变大都要重算）。
        // 放在建视图之前：后面所有 PER_ROW 的使用都依赖它已经是正确值。
        updateColumns()
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            // **根视图必须有不透明背景。**
            //
            // 输入法窗口本身是透明的（系统不给它画底色），哪个子 View 没铺背景，
            // 那里就直接透出下层应用的界面。之前 root 没设 backgroundColor，
            // 只有 keyboardHost 铺了，于是候选条一旦有空隙（候选少、临时隐藏、
            // 展开态接管时的那一帧），就会看到宿主应用的内容从候选条里"穿"出来。
            // 用户描述是「候选框被软件里的内容盖住了」。
            //
            // 铺在 root 上而不是逐个设：子 View 增删、显隐切换都不会再漏一处。
            setBackgroundColor(Palette.of(this@FeiheIme).keyboardBg)
            // **高度兜底，防「展开候选后整个输入法消失」。**
            //
            // 展开态会把键盘整块 GONE 掉，只剩候选区。如果候选区高度算小了
            // （root 还没 measure 完、expandedRows() 返回 1、或 window 恰好在
            // 刷新中途为空），root 高度会塌到 0。系统看到输入法视图没有高度，
            // 就认定这个 IME 没什么可显示，直接收掉窗口 —— 表现为
            // `onStartInput: inputType=0x0`、焦点丢失、整块屏幕只剩背景。
            //
            // 给一个最小高度，让任何状态下的 root 都还是「有东西的」。
            // 取 120dp：大于一行候选(40) + 收起条(30)，小于正常键盘高度，
            // 不会影响正常布局（正常状态 root 本来就比它高）。
            minimumHeight = dp(120f)
        }

        statusText = TextView(this).apply {
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 12f)
            gravity = Gravity.CENTER_VERTICAL
            // 上下只留 3dp：文案已经不常驻了（见 refreshStatus），留 6dp 会撑着
            // 一条偏高的空条。左右 12dp 改成 10dp，让组合码尽量靠左不被截断。
            setPadding(dp(10f), dp(3f), dp(10f), dp(3f))
        }

        // 状态条右侧的齿轮。**输入法必须有这个入口**：系统设置里的那一层藏得深，
        // 用户第一次装完根本不知道去哪调方案。
        // Palette 只在这里取一次：apply{} 里的  是 TextView，不是 Context
        val statusPalette = Palette.of(this)

        // 状态条上全是图标按钮（齿轮/剪贴板/麦克风/收起）。**必须统一带框**：
        // 没框的图标和候选文字混在一起时，用户分不清哪个能点。
        // 这几个原来全是裸 emoji/字符，用户反馈「收起键盘的小按钮也没有边框」——
        // 其实四个都没有，只是那个最刺眼。统一走这个工厂，别各写一份。
        fun iconButton(text0: String, desc: String, sizeSp: Float = 18f): TextView =
            TextView(this).apply {
                text = text0
                setTextSize(TypedValue.COMPLEX_UNIT_SP, sizeSp)
                gravity = Gravity.CENTER
                setPadding(0, 0, 0, 0)
                setTextColor(statusPalette.sub)
                background = android.graphics.drawable.GradientDrawable().apply {
                    setColor(statusPalette.chipBg)
                    setStroke(dp(CHIP_STROKE_DP), statusPalette.chipStroke)
                    cornerRadius = dp(4f).toFloat()
                }
                // 显式尺寸：minWidth 在按内容测量的场景下不可靠（实测过 w=0）。
                layoutParams = LinearLayout.LayoutParams(dp(44f), dp(30f)).apply {
                    marginStart = dp(4f)
                }
                contentDescription = desc
            }

        val gear = iconButton("⚙", "输入法设置").apply {
            setOnClickListener { openSettings() }
        }

        // 剪贴板历史入口。放在状态条上而不是候选条里：候选条只在有候选项时才有意义，
        // 而剪贴板随时可能要用（复制了链接、地址、验证码之后）。
        clipButton = iconButton("📋", "剪贴板历史", 17f).apply {
            setOnClickListener { toggleClipboard() }
        }

        // 麦克风：**单击开始、再单击结束**。
        //
        // 以前是按住说话（OnTouch DOWN 开录 / UP 停录），但状态条上的按钮离
        // 屏幕底边很近，长按容易滑出去触发 ACTION_CANCEL；而且用户腾不出另一只手
        // 去点「结束」。改成两态开关后，录多久都行，状态条会显示「正在录音」。
        micButton = iconButton("🎤", "语音输入", 17f).apply {
            visibility = if (voiceEnabled) View.VISIBLE else View.GONE
            setOnClickListener { toggleVoice() }
            // 长按 = 撤销上一次语音纠错。
            //
            // 不另开一个按钮：语音纠错是低频事件，专门占一格会挤掉常用键。
            // 长按麦克风是「刚才那句说错了」的直觉动作，且只在有可撤销内容时生效
            // （没纠错过时长按什么都不做，不会误触）。
            setOnLongClickListener {
                if (voiceUndo == null) {
                    false
                } else {
                    undoVoiceCorrect()
                    Toast.makeText(this@FeiheIme, "已还原语音原文", Toast.LENGTH_SHORT).show()
                    true
                }
            }
        }

        // 表情入口。原来在字母页底排（123 和空格之间），用户要求搬到状态条：
        // 底排那个位置太窄、又只在字母页可见；放进状态条后任何页都能点，
        // 且底排腾出的宽度还给了空格（盲按更不容易误触）。
        emojiButton = iconButton("😊", "表情", 17f).apply {
            setOnClickListener { showPage(Page.EMOJI) }
        }

        // 收起键盘。放在最右边（拇指最远但按一次就消失，不需要高频）。
        //
        // 系统键盘底部有自己的收起按钮，但它是**系统手势条**上的小箭头，
        // 各家 ROM 位置和样式不一致，有的还藏起来。自带一个更可靠：
        // 尤其在候选条展开、键盘被 GONE 掉的时候，这是唯一的退路。
        hideButton = iconButton("⌄", "收起键盘").apply {
            setOnClickListener { requestHideSelf(0) }
        }

        statusBar = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setBackgroundColor(statusPalette.statusBg)
            addView(
                statusText,
                LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f),
            )
            addView(clipButton)
            addView(emojiButton)
            addView(micButton)
            addView(gear)
            addView(hideButton)
        }
        root.addView(
            statusBar,
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.WRAP_CONTENT,
            ),
        )

        // 候选条：**纵向容器**，收起时只有一行（横向铺开、可横滑），展开时最多 3 行。
        // **不能是 Dialog**——输入法窗口层级特殊，Dialog 会被系统盖住；它必须是
        // InputView 的一部分。
        //
        // 为什么要纵向容器而不是「一个 LinearLayout 按需改成 VERTICAL」：orientation
        // 改了之后所有子 View 的 LayoutParams 语义都变了（weight 从横向变纵向），
        // 复用池会错位。分成「容器 + 若干行」两层，每行永远是横向的，复用逻辑不变。
        // 候选容器。收起态是「一行横滑」，展开态是「多行静态铺开」——两种形态
        // 对父容器的要求是冲突的：
        //
        //   HorizontalScrollView 会给子 View **无限宽度**（UNSPECIFIED）。在这种
        //   约束下，行内 MATCH_PARENT 退化成 WRAP_CONTENT，weight=1 的候选格
        //   量不出宽度直接塌成 0 —— 展开后看起来「只有一行且挤成一团」。
        //
        // 所以展开态把候选区挂到 root 的另一个槽位（expandedSlot，普通
        // LinearLayout，宽度跟着屏幕），收起态才用 candidateScroll。
        // 两套容器各自 measure 正确，不用互相妥协。
        candidateColumn = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
        }
        candidateScroll = HorizontalScrollView(this).apply {
            isHorizontalScrollBarEnabled = false
            addView(candidateColumn)
        }

        // 展开态专用槽位：和 candidateScroll 二选一显示，宽度 MATCH_PARENT。
        // 它自己不是 ScrollView，纵向多行由它直接测量。
        expandedColumn = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            visibility = View.GONE
        }
        root.addView(
            candidateScroll,
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                dp(CANDIDATE_ROW_DP),
            ),
        )
        // 展开态容器和 candidateScroll 并列、同一个高度槽位；谁可见由
        // applyExpandedChrome 决定，另一个 GONE（GONE 不占高度）。
        //
        // **外面必须再套一层纵向 ScrollView**：
        // 以前 expandedColumn 直接挂 root，高度是「算出来能放几行就多高」，
        // 而行数由 expandedRows() 靠 root 高度估算——算多了最后一行就被 root
        // 边界裁掉半截（用户看到的「最后一行显示不完整」），算少了下方留空。
        // 套上 ScrollView 之后，行数不再是生死问题：放不下就滚。这让
        // 「候选到底有几条」「这机器屏幕多高」都不再影响正确性。
        expandedScroll = ScrollView(this).apply {
            isVerticalScrollBarEnabled = true
            // 滚到边界时不要多出蓝色光晕，输入法窗口里那个效果很跳
            overScrollMode = View.OVER_SCROLL_NEVER
            visibility = View.GONE
            // **滚到底部自动追加下一批。**
            //
            // 展开态是懒加载：先渲染 60 个（约 18ms，一帧内），用户滑到底时
            // 再追加 60 个。不这么做就只有两种选择——一次渲染 565 个（卡 170ms），
            // 或者分 14 页让用户翻（比滚动累得多）。
            //
            // 判据用「还剩不到两行就追加」而不是「精确到底」：精确到底时用户
            // 已经看到空白了，追加完再跳一下，观感是「卡了一下」。提前一个
            // 生产批次，滚动是连续的。
            setOnScrollChangeListener { _, _, scrollY, _, _ ->
                val col = expandedColumn ?: return@setOnScrollChangeListener
                val contentBottom = col.height
                val viewBottom = scrollY + height
                val nearBottom = contentBottom - viewBottom < dp(CANDIDATE_ROW_DP) * 2
                if (nearBottom && expandedLoaded < candidates.size) {
                    expandedLoaded = (expandedLoaded + CandidateGrid.EXPAND_BATCH)
                        .coerceAtMost(candidates.size)
                    // 追加批次时**不能**把滚动位置归零，否则用户滑到底就被打回顶部，
                    // 永远看不完后面的候选。这里直接重渲染展开区（不走 renderCandidates，
                    // 免得把展开的 chrome/键盘状态又重算一遍）。
                    expandedColumn?.let { c ->
                        renderExpandedRows(c, visibleCandidates(), resetScrollHere = false)
                    }
                }
            }
            addView(
                expandedColumn,
                // FrameLayout.LayoutParams：ScrollView 是 FrameLayout 的子类，
                // 用 ViewGroup.LayoutParams 它量不出内容高度（见下面 setLayoutParams
                // 处的注释）。
                FrameLayout.LayoutParams(
                    FrameLayout.LayoutParams.MATCH_PARENT,
                    FrameLayout.LayoutParams.WRAP_CONTENT,
                ),
            )
        }
        root.addView(
            expandedScroll,
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                dp(CANDIDATE_ROW_DP),
            ),
        )

        // 录音波形：占用和候选条**同一个槽位**（第三个并列子 View）。
        //
        // 不和候选条叠放、也不插在候选条上下：录音时用户不看候选，
        // 而候选条的高度是固定的（CANDIDATE_ROW_DP），波形用同一格高度
        // 就不会让整个键盘在录音瞬间跳一下。
        //
        // 三个容器永远最多一个可见：候选（收起）/ 展开候选 / 波形。
        voiceWave = com.feihe.ime.voice.VoiceWaveView(this).apply {
            visibility = View.GONE
            setPadding(dp(10f).toInt(), dp(6f).toInt(), dp(10f).toInt(), dp(6f).toInt())
            setTint(Palette.of(this@FeiheIme).accent)
        }
        root.addView(
            voiceWave,
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                dp(CANDIDATE_ROW_DP),
            ),
        )

        // 数字选词行：夹在候选条和键盘之间。没候选时整行 GONE（高度 0），
        // 所以它不占字母键的高度——这是 A2「不挤掉字母高度」的落点。
        numberRowKey = Int.MIN_VALUE
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

        // 展开态的「收起」条：默认隐藏，只在候选展开时出现。
        // 它必须在键盘**前面**（更靠上）——键盘会被 GONE 掉，这条是唯一退路。
        collapseBar = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            // 内容居中：条本身是整宽（点哪儿都能收），但**胶囊框**跟着文字走。
            gravity = Gravity.CENTER
            visibility = View.GONE
            // 垂直不留 padding：高度由外层 LayoutParams 钉成 CANDIDATE_ROW_DP，
            // 这里再加 6dp 的话实际是 52dp，药丸就跟候选行对不齐了。
            setPadding(dp(8f), 0, dp(8f), 0)
            setOnClickListener {
                candidateExpanded = false
                renderCandidates()
                refreshStatus()
            }
            // **给「收起」一个看得见的外框。**
            //
            // 以前它只是一行 13sp 的灰字孤零零浮在空白区顶上，用户反馈「看着很单薄」
            // ——确实是：没有边界的东西在同一屏里既不像按钮、也不知道点哪里有效。
            // 展开态下这块区域是整块空白（键盘被藏了），一行没有容器的字就更像
            // 排版残留而不是可点控件。
            //
            // 所以给一个**胶囊描边**：圆角 = 高度的一半（药丸形），细描边 + 透明底，
            // 不抢候选的视觉重心，但明确圈出「这是能点的东西」。
            // 描边用 stroke 色而不是强调色：收起是次要动作，红色/蓝色会喧宾夺主。
            addView(LinearLayout(this@FeiheIme).apply {
                orientation = LinearLayout.HORIZONTAL
                gravity = Gravity.CENTER
                // 内容不接收点击，让事件冒到外层整宽条（点空白也能收）
                isClickable = false
                isFocusable = false
                val pal = Palette.of(this@FeiheIme)
                background = android.graphics.drawable.GradientDrawable().apply {
                    cornerRadius = dp(15f).toFloat()
                    // **不调 setColor(TRANSPARENT)。**
                    //
                    // 这里是「收起按钮没有边框」的真正原因：GradientDrawable
                    // 不设 color 时本来就是透明填充，显式设一个全透明色会在部分
                    // Android 版本上让整张 shape 的绘制被跳过，描边跟着一起消失。
                    // 框不是「太淡看不见」，是**根本没画出来**。
                    //
                    // 顺带把描边色从 stroke(0xFF9AA0A8) 换成 chipStroke：
                    // 前者是分隔线用的亮灰，画在键盘底色上对比度不够；
                    // 后者是给小控件准备的，深一档。
                    setStroke(dp(CHIP_STROKE_DP), pal.chipStroke)
                }
                // 左右留白比上下大：药丸需要横向余量才像按钮。
                // 垂直方向要撑满整行高度（见下面 collapseBar 的 40dp），
                // 只靠 7dp padding 的话实际高度约 33dp，手指点着发虚。
                setPadding(dp(16f), 0, dp(16f), 0)
                addView(TextView(this@FeiheIme).apply {
                    text = "▴ 收起候选"
                    setTextSize(TypedValue.COMPLEX_UNIT_SP, 14f)
                    gravity = Gravity.CENTER
                    setTextColor(pal.sub)
                })
            })
        }
        root.addView(
            collapseBar,
            // 高度钉成一行候选的高度：药丸跟着撑满，点哪都能收。
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                dp(CANDIDATE_ROW_DP),
            ),
        )

        keyboardHost = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
        }
        root.addView(keyboardHost)

        // 剪贴板面板：和键盘同层、同尺寸，平时 GONE，点状态条上的剪贴板按钮时
        // 把键盘藏起来、把它露出来。
        //
        // **它必须真的被创建**。之前 clipPanel / clipList 两个字段只被 setVisibility
        // 和 renderClipboard 用过，`= LinearLayout(...)` 一次都没有——对象永远是
        // null。于是点按钮的效果是「键盘藏起来了，但没有任何东西顶上来」，
        // 整个输入区一片空白，用户看到的就是「点了没反应」。
        clipList = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
        }
        clipPanel = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            visibility = View.GONE
            setBackgroundColor(Palette.of(this@FeiheIme).bg)
            // 顶部一行「剪贴板」标题 + 关闭，下面才是列表
            addView(
                LinearLayout(this@FeiheIme).apply {
                    orientation = LinearLayout.HORIZONTAL
                    gravity = Gravity.CENTER_VERTICAL
                    setPadding(dp(12f), dp(8f), dp(12f), dp(8f))
                    addView(
                        TextView(this@FeiheIme).apply {
                            text = "剪贴板"
                            setTextSize(TypedValue.COMPLEX_UNIT_SP, 14f)
                            setTextColor(Palette.of(this@FeiheIme).sub)
                        },
                        LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f),
                    )
                    addView(
                        TextView(this@FeiheIme).apply {
                            text = "✕"
                            setTextSize(TypedValue.COMPLEX_UNIT_SP, 16f)
                            setTextColor(Palette.of(this@FeiheIme).sub)
                            setPadding(dp(12f), dp(4f), dp(4f), dp(4f))
                            contentDescription = "关闭剪贴板"
                            setOnClickListener { closeClipboard() }
                        },
                    )
                },
                LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.MATCH_PARENT,
                    LinearLayout.LayoutParams.WRAP_CONTENT,
                ),
            )
            // 列表本体。长了好歹能滚：条数上限是 ClipHistory.LIMIT，但每条可能是
            // 多行文本压成一行，屏小的时候仍然会顶出去。
            addView(
                android.widget.ScrollView(this@FeiheIme).apply {
                    isFillViewport = true
                    addView(clipList)
                },
                LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.MATCH_PARENT,
                    0,
                    1f,
                ),
            )
        }
        root.addView(
            clipPanel,
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                0,
                1f,
            ),
        )
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
        // 双击窗口也一起清：切页本来就把 ⇧ 归零了，不该让「切页前那一下」
        // 和「切回来后第一下」凑成一次双击。
        lastShiftTapAt = 0L
        host.removeAllViews()
        host.addView(
            KeyboardView.build(
                this,
                KeyboardPages.rows(target, chinese),
                shiftState,
                KeyboardPages.rowHeightDp(target) * UiPrefs.keyScale(this),
                // **形码态才传方案名**：非 null 时按键换成「字母小字 + 形码提示大字」，
                // 并且标点/字母退居次要位置。退出形码态（assistActive=false）传 null，
                // 键盘立刻恢复原样——这也是「退出形码就还原」的实现点。
                assistMode = if (assistActive && assistMode != "off") assistMode else null,
                onLongKey = { onLongKey(it) },
                // 空格键松手：如果这次长按已经进了语音，抬起就结束录音。
                //
                // 注意也要处理 LOADING：模型冷加载要几秒，用户可能已经松手了，
                // 此时 voiceState 还停在 LOADING，录音线程刚起来就要收工。
                // 直接调 stop() 会把 recording 标志清掉，线程进循环立刻退出，
                // 结果是「按了、说了、什么也没发生」。
                onVoiceUp = {
                    if (voiceState == VoiceRecorder.State.RECORDING ||
                        voiceState == VoiceRecorder.State.LOADING
                    ) {
                        voice?.stop()
                    }
                },
            ) { onKey(it) },
        )
        // 建完就把中/英键的字刷成当前模式：换页重建后不能又变回「中」
        KeyboardView.applyModeState(host, chinese)
        refreshStatus()
    }

    private fun openSettings() {
        // 输入法进程里启动 Activity 必须带 NEW_TASK：这个 Context 不是一个 Activity。
        val i = Intent(this, SettingsActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        runCatching { startActivity(i) }
    }

    /**
     * 麦克风按钮：单击开始录音，再单击结束录音并识别。
     *
     * 只有 [VoiceRecorder.State.RECORDING] 需要「再点一下结束」；LOADING 阶段用户
     * 可能想取消（模型冷加载要几秒），也允许点击中止。其余状态点击都是「开始」。
     */
    private fun toggleVoice() {
        val v = voice ?: return
        val state = voiceState
        if (state == VoiceRecorder.State.RECORDING) {
            // 再点一下：停止录音，剩下的交给识别
            v.stop()
            return
        }
        if (state == VoiceRecorder.State.LOADING || state == VoiceRecorder.State.RECOGNIZING) {
            // 加载/识别中再点：不重复开录，避免两个线程抢 AudioRecord
            return
        }
        voiceWave?.resume()
        v.start()
    }

    /** 语音状态 → 状态条提示。权限/模型缺失给一句能照做的话，不静默失败。 */
    private fun onVoiceState(s: VoiceRecorder.State) {
        voiceState = s
        val listening = s == VoiceRecorder.State.RECORDING
        val busy = listening || s == VoiceRecorder.State.RECOGNIZING || s == VoiceRecorder.State.LOADING
        micButton?.text = when {
            listening -> "⏹"
            busy -> "…"
            else -> "🎤"
        }
        // 波形只在这三态下露头：正在收音/正在识别/加载模型。
        // 其余状态（空闲、报错、没权限）藏起来，让候选条回到正常位置——
        // 一直挂着一条静止的波形会让用户以为还在听。
        voiceWave?.visibility = if (busy) View.VISIBLE else View.GONE
        // 录音时不显示候选：候选条和波形是同一个高度槽位，同时出现会叠在一起。
        // 这里的 visibility 会被 updateTopSlot 覆盖，所以 updateTopSlot 里也要认
        // 「正在录音」这个条件（见那里的注释）。
        if (busy) {
            candidateScroll?.visibility = View.GONE
            expandedColumn?.visibility = View.GONE
        }
        when (s) {
            VoiceRecorder.State.LOADING -> refreshStatus("正在加载语音模型…")
            // 改成交互开关后，提示语必须改：以前是「松手上屏」，现在松手什么也不会发生
            VoiceRecorder.State.RECORDING -> refreshStatus("正在录音，再点一下结束")
            VoiceRecorder.State.RECOGNIZING -> {
                refreshStatus("正在识别…")
                // 识别阶段不再收音，波形定格并调暗——这是「我说完了、它在算」的信号
                voiceWave?.freeze()
            }
            VoiceRecorder.State.NO_PERMISSION -> {
                refreshStatus("需要麦克风权限，请在设置里开启语音输入")
                Toast.makeText(this, "请先授权麦克风权限", Toast.LENGTH_SHORT).show()
            }
            VoiceRecorder.State.NO_MODEL -> refreshStatus("语音模型缺失")
            VoiceRecorder.State.FAILED -> {
                // 把引擎给的原因带出来。「语音识别失败」这五个字对用户没有任何
                // 行动价值——他既不知道是没权限、模型坏了，还是引擎崩了。
                val why = VoiceEngine.lastError()
                refreshStatus(if (why.isNullOrBlank()) "语音识别失败" else "语音识别失败：$why")
            }
            VoiceRecorder.State.EMPTY -> refreshStatus("没听到声音，再说一次")
            VoiceRecorder.State.IDLE -> refreshStatus()
        }
    }

    /**
     * 对语音识别结果做自造词纠错。
     *
     * 开关默认关（见 [UiPrefs.voiceCorrect]）：这功能会改写用户说的话，
     * 不主动打开就一次都不该触发。关着时原样返回，调用方据此判断要不要留撤销入口。
     *
     * 判据全在 [com.feihe.ime.voice.VoiceCorrect] 里（拼音完全吻合、长度一致），
     * 这里只负责「把自造词库和查拼音的能力喂给它」。
     */
    private fun correctVoiceText(text: String): com.feihe.ime.voice.VoiceCorrect.Result {
        val unchanged = com.feihe.ime.voice.VoiceCorrect.Result(text, text)
        if (!UiPrefs.voiceCorrect(this)) return unchanged
        if (text.isEmpty()) return unchanged
        // 自造词库就是「用户天天用的词」，ASR 最容易认错的正是它们
        val phrases = NativeEngine.phrases().map {
            com.feihe.ime.voice.VoiceCorrect.Phrase(it.first, it.second)
        }
        if (phrases.isEmpty()) return unchanged
        // 逐字读音**只查一次**：把原文里所有汉字一次交给引擎批量反查，
        // 比每个字调一次 JNI 快一个量级（纠错在识别完成的回调里跑，
        // 卡住就是「点了麦克风半天没反应」）。
        val readings = NativeEngine.readings(text)
        val of = com.feihe.ime.voice.VoiceCorrect.PinyinOf { ch ->
            // 多音字取第一条：纠错只做「完全吻合」，取错读音的后果是**这条不纠**，
            // 而不是纠错——安全的失败方向。
            readings[ch]?.firstOrNull() ?: ""
        }
        return com.feihe.ime.voice.VoiceCorrect.correct(text, phrases, of)
    }

    /**
     * 撤销上一次语音纠错：把纠正过的文字还原成 ASR 原文。
     *
     * ## 为什么用 deleteSurroundingText 而不是选区块
     *
     * `setSelection` 收的是**编辑器里的绝对下标**，而输入法不知道「刚上屏的那段」
     * 从第几个字符开始（宿主可能在任何位置，富文本里还有格式标记）。
     * `deleteSurroundingText(前删, 后删)` 是相对**光标**的，光标刚在纠错词的末尾，
     * 往前删 len 个正好是那段——不需要知道绝对位置。
     *
     * 纠错前后**长度必然相同**（这是 VoiceCorrect 的硬约束），所以删 len 补 len
     * 不会让后续文字错位。
     */
    private fun undoVoiceCorrect() {
        val u = voiceUndo ?: return
        voiceUndo = null
        if (!u.changed) return
        val ic = currentInputConnection ?: return
        val len = u.text.length
        if (len <= 0) return
        ic.deleteSurroundingText(len, 0)
        ic.commitText(u.original, 1)
        refreshEmailSuggest()
    }

    /**
     * 点了英文单词建议：用它**替换**输入框里已经上屏的那串字母。
     *
     * ## 为什么不能再用 commitText 覆盖
     *
     * 以前英文是「组合态」（`setComposingText`），`commitText` 会自动替换整个
     * 组合区，正好是「把 hel 换成 hello」。
     *
     * 现在英文改成**直接上屏**（见 [pushComposingEnglish] 的说明），组合区根本不存在，
     * `commitText` 就变成**插入**——用户会得到 `helhello`。
     * 所以必须自己回删：先删掉光标前那 `pending.length` 个字符，再写建议词。
     */
    private fun pickEnglishWord(word: String) {
        val ic = currentInputConnection ?: return
        val typed = pending.length
        if (typed > 0) {
            // 先删自己刚上屏的那些字母，再写建议词。
            // 顺序不能反：先写后删会把刚写进去的也删掉。
            ic.deleteSurroundingText(typed, 0)
        }
        ic.commitText(word, 1)
        pending.setLength(0)
        englishWords = emptyList()
        englishSuggest?.query("")
        refreshCandidates()
        refreshStatus()
        forgetContext()
    }

    private fun onKey(key: Key) {
        // 用户开始敲键了 → 上一次语音纠错的撤销窗口关闭。
        //
        // 撤销靠「光标前 len 个字符」定位，一旦用户又输入了别的东西，
        // 那个位置就不再是纠错词了，这时撤销会改到不该动的文字上。
        // 宁可让他撤销失败，也不能改错。
        voiceUndo = null
        when (key.kind) {
            KeyKind.CHAR -> {
                // 形码态：字母归形码，不延长编码。打满之后多按的直接吞掉——放它去
                // 延长编码会重新分段/重查，刚筛出来的那一段就没了（桌面端同一条规则）。
                if (assistActive) {
                    if (assistKeys.length < assistMaxKeys()) {
                        assistKeys.append(key.code)
                        refreshAssist()
                    }
                    return
                }
                // **联想在打字时保留**（原来一敲字母就清掉）。那个老决定的理由是
                // "留着旧联想等于把几分钟前的上下文摆在手指下面"，但现在两处已经变了：
                // ① onStartInput 会读宿主前文重设 prevWord，切窗口后不再是陈旧的上下文；
                // ② 每次 commitWord 都会按刚上屏的词重算 suggestions。
                // 保留它才能让候选条末尾的 →词 真正点得到，也和搜狗一致。
                // 代价是候选和联想同屏——现在候选条不再限数量，联想直接排在候选后面，
                // 横滑就能看到，不需要再为它压缩候选。
                if (ShiftMachine.upper(shiftState)) {
                    // 一次性打完就落下；锁定态保持。状态迁移在 ShiftMachine 里（有单测）。
                    shiftState = ShiftMachine.afterChar(shiftState)
                    KeyboardView.applyShiftState(keyboardHost, shiftState)
                    if (chinese) {
                        // 中文态 + 上档 = **临时英文**：先把手上半截拼音处理掉再上屏，
                        // 否则「nihao」+ ⇧A 会变成组合态里的「nihaoA」。
                        // 上屏的这个大写字母也不参与联想（上下文已经断了）。
                        if (candidates.isNotEmpty()) {
                            commitWord(visibleCandidates().first(), thenSuggest = false)
                        } else {
                            commitPending()
                        }
                        commitText(key.code.uppercase())
                        forgetContext()
                        return
                    }
                    // 英文态 + 上档：**直接上屏大写字母**。
                    //
                    // 以前这里走 `pending.append` 把大写字母塞进组合串，于是
                    // 候选条上出现一个「A」候选，用户还得再按一次空格才上屏。
                    // 英文输入没有「组合串」这个概念——按了就该立刻出现。
                    commitText(key.code.uppercase())
                    forgetContext()
                    return
                }
                // 快捷输入的引导符 `/`：只在「中文态 + 手上没有别的组合串」时进入。
                // 别的字母还在组合中（`nihao` 打到一半）时按 `/`，应当照旧上屏拼音
                // 再打斜杠，否则会凭空把用户打了一半的拼音吞掉。
                if (chinese && pending.isEmpty() && key.code == NativeEngine.quickPrefix()) {
                    // /fc 要读剪贴板：进快捷输入前强制重读一次，否则用户刚复制的字
                    // 不在引擎里（键盘弹出时读的那份可能已经旧了）。
                    readClipboard(force = true)
                    quickMode = true
                    quickWords = emptyList()
                    pushComposing()
                    return
                }
                // **英文态：字母直接上屏（不走组合态）。**
                //
                // 原来是 setComposingText 组合态（豆包那种带下划线、可被候选词替换）。
                // 改成直接上屏的唯一原因是 **Termux 这类终端宿主不渲染组合文本**：
                // 用户在 Termux 命令行里打英文一个字都看不见，按空格才整串冒出来。
                // 用户明确反馈了这个场景，而输入法在终端里打字是刚需。
                //
                // 代价见 [pushComposingEnglish]：点建议词改成「回删 + 重写」。
                //
                // pending 仍然攒着，它是「已打进宿主的那串字母」的账本，
                // 退格按它的长度回删、点建议按它的长度替换。
                if (!chinese) {
                    pending.append(key.code)
                    pushComposingEnglish()
                    return
                }
                pending.append(key.code)
                pushComposing()
            }

            KeyKind.DELETE -> {
                // **英文组合态：退格删掉未确认的最后一个字母。**
                //
                // 必须放在最前面：中文那条路会「整词 → 单字」拆段，对英文字母
                // 毫无意义，硬走会把 hello 变成一个莫名其妙的候选状态。
                //
                // 删空之后不再继续删宿主的字符——组合区已经空了，再删会吃掉
                // 用户之前打的空格或上一个词。
                if (!chinese) {
                    if (pending.isNotEmpty()) {
                        pending.setLength(pending.length - 1)
                        // 英文是**直接上屏**的，退格就是真删——不再有任何组合区
                        // 需要靠「setComposingText("")」去撤下划线。
                        deleteOneCharBackward()
                        if (pending.isEmpty()) {
                            englishWords = emptyList()
                            englishSuggest?.query("")
                        }
                        refreshCandidates()
                        refreshStatus()
                    } else {
                        deleteOneCharBackward()
                        forgetContext()
                    }
                    return
                }
                // 形码态：先删形码；删空之后再按一次退出形码态（不回退去删拼音）。
                if (assistActive) {
                    if (assistKeys.isNotEmpty()) {
                        assistKeys.deleteCharAt(assistKeys.length - 1)
                    } else {
                        // 形码删空后退出形码态 → 键盘要还原成正常字母/标点。
                        // 走 resetAssist()：它统一负责「退出就重建键盘」，别在这里
                        // 再手写一遍 assistActive=false + rebuildKeyboard（两套会漂）。
                        resetAssist()
                    }
                    refreshAssist()
                    return
                }
                // 快捷输入态：退格删码；删到只剩引导符时再按一下，整个退出。
                // 不能在这里「删完就直接退出」——`/uui` 删回 `/` 时用户还想重打，
                // 突然退出会和「打一半字消失」一样莫名其妙。
                if (quickMode) {
                    if (pending.length > 1) {
                        pending.setLength(pending.length - 1)
                        pushComposing()
                    } else {
                        quickMode = false
                        quickWords = emptyList()
                        pushComposing()
                    }
                // 展开某段时，退格先收起该段（和「返回」一个意思），别直接删拼音。
                // chip 视图下则照常删一个字母——用户还在编辑那串码。
                } else if (segmentState != null && segmentState!!.focus >= 0) {
                    segmentState = segmentState!!.back()
                    renderCandidates()
                } else if (pending.isNotEmpty()) {
                    // **退格 = 整词 → 单字（逐段选），不删编码。**
                    //
                    // 这是和 Windows 端一致的手感（`textservice.rs` 的
                    // `backspace_into_segments`）：打 `xrji` 出「璇玑」，但用户想要
                    // 「璇」+ 别的字，按一次退格就切成 `[璇][玑]`，两边各自能换字；
                    // 编码 `xrji` 一个字都不少。
                    //
                    // 判据也和 Windows 一样（`backspace_splits`）：
                    //   - 第一候选是**多字词** → 进逐字，说明这码被一个词占满
                    //   - 第一候选是单字（`de` → 的）→ 这码本来就是一串同码字，不劫持
                    //   - 没有候选 → 退回删字母
                    //
                    // **已在分段态（chip 视图）时，退格要退出去删字母。**
                    // 用户的原话是「候选里确实没有想要的，可能是打错了，
                    // 要收起再用退格改输入字母」——这是分段态的**唯一出口**：
                    // 没有它，用户进了逐字选词就再也改不了码，只能全删重打。
                    // 注意要连 `segmentState` 一起清，否则它指向旧的段，
                    // 下一轮 `backspaceIntoSegments()` 会因为「已在分段态」直接返回 false。
                    if (segmentState != null) {
                        segmentState = null
                        segmentStateCode = ""
                        pending.setLength(pending.length - 1)
                        pushComposing()
                    } else if (!backspaceIntoSegments()) {
                        pending.setLength(pending.length - 1)
                        pushComposing()
                    }
                } else {
                    // 组合串空了才删文档里的字。反过来（先删文档）会把用户打了一半的
                    // 拼音吃掉，这是最容易被骂的行为之一。
                    //
                    // **必须按 code point 删，不能固定删 1**：emoji 是代理对，占两个
                    // UTF-16 code unit。deleteSurroundingText(1, 0) 只会删掉代理对的
                    // 后半截，留下半个字符（显示成乱码），用户得再按一次删除才干净——
                    // 这就是「emoji 退格只删一半」。
                    deleteOneCharBackward()
                    // 删掉了一个字，上一个词是什么已经说不清了
                    forgetContext()
                }
            }

            KeyKind.SPACE -> {
                // 快捷输入态：空格 = 选第一个结果（和拼音选词同一个肌肉记忆）。
                // 还没选出任何结果（只打了 `/` 或者码还没成命令）时，把码原样上屏——
                // 用户想打「/」这个字符本身也走这条路，比弹一句「无效命令」友好。
                if (quickMode) {
                    if (quickWords.isNotEmpty()) commitQuick(quickWords.first()) else commitPending()
                    return
                }
                // **英文组合态：空格 = 确认原文，再补一个空格。**
                //
                // 这是豆包的行为：打了 hel 还没选候选就按空格，那就认下 hel
                // 然后往后空一格。用户不需要额外操作「确认」。
                //
                // 先 commit 再打空格，两步分开：合成一次 commitText("hel ")
                // 虽然结果一样，但组合态的语义要求先把未确认区结束掉，
                // 某些宿主（WebView）对「带空格一起 commit」处理不一致。
                if (!chinese && pending.isNotEmpty()) {
                    commitEnglishComposing()
                    commitText(" ")
                    forgetContext()
                    return
                }
                // 分段态：空格 = **确认当前段，前进到下一段**（等价于点当前候选）。
                //
                // ## 为什么要这么改（用户原话）
                // 「长任务这三个字，我想用空格确认第一个字，实际直接三个字都用第一个
                //   候选上屏了。」——以前这里是 `commitSegmentIfActive()`，一下把整句
                // 拼好上屏，用户在逐字挑选时按空格等于放弃了后面所有选择机会。
                //
                // 点候选是逐段确认的，空格应该给同一个语义：这是所有输入法的肌肉记忆
                // （打一个字 → 空格确认 → 接着打下一个）。只有在**最后一段**选完，
                // 空格才承担「整句上屏」——那时确实没有下一段可去了。
                if (commitSegmentStepOrFinish()) return
                if (pending.isEmpty()) {
                    commitText(" ")
                    // 空格断开上下文：下一个词和上一个词之间没有直接关系
                    forgetContext()
                } else if (chinese && candidates.isNotEmpty()) {
                    // 中文态空格 = 选第一个。这是所有拼音输入法的肌肉记忆，
                    // 不这么做用户会以为候选是装饰品。
                    commitWord(visibleCandidates().first())
                } else {
                    // 没有候选（码还没成词，或者这是个英文串）→ 原样上屏。
                    // **不能什么都不做**：那样用户打了一串字母就卡住，只能一个个删。
                    commitPending()
                }
            }

            KeyKind.PUNCT -> {
                // 快捷输入态：和拼音同一个次序——先把命令结果上屏，再粘标点。
                // 这样「/rq，」得到「2024-06-01，」，而不是「/rq」+「，」。
                if (quickMode) {
                    if (quickWords.isNotEmpty()) commitQuick(quickWords.first()) else commitPending()
                    commitText(key.code)
                    forgetContext()
                    return
                }
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
                if (quickMode) quickMode = false
                // 回车规格全在 EnterSpec 里（纯函数，EnterSpecTest 逐条钉住）。
                // 这里只负责按判定执行，**不要再自己写 if 判断**——
                // 这段逻辑改坏过三次，每次都是某个默认分支选反。
                when (val dec = EnterSpec.decide(
                    chinese = chinese,
                    pendingNotEmpty = pending.isNotEmpty(),
                    imeAction = fieldAction,
                    multiline = isMultilineField(),
                )) {
                    EnterSpec.Decision.CommitOnly -> {
                        // 手上有输入：原样上屏就结束，不换行、不发 action。
                        //   中文态 → 那串字母直接 commitText
                        //   英文态 → 字母早就逐字上屏了，这里只清账本
                        if (!chinese) commitEnglishComposing() else commitPending()
                    }
                    is EnterSpec.Decision.PerformEditorAction -> {
                        // 没输入 + 宿主有明确动作（地址栏 GO / 搜索框 SEARCH）
                        currentInputConnection?.performEditorAction(dec.action)
                    }
                    EnterSpec.Decision.Newline -> commitText("\n")
                    // DoNothing 不存在于返回路径（测试里断言了），
                    // when 仍需穷尽，给个空分支
                    EnterSpec.Decision.DoNothing -> Unit
                }
                forgetContext()
            }

            KeyKind.MODE -> {
                // 切中英 = 上下文变了：形码态必须退掉，否则英文态下打的字母还会被
                // 当成形码吃掉（形码只在中文态有意义）。
                resetAssist()
                chinese = !chinese
                shiftState = ShiftState.OFF
                lastShiftTapAt = 0L
                KeyboardView.applyShiftState(keyboardHost, shiftState)
                // 键上的字要跟着变：中文态「中」，英文态「英」
                KeyboardView.applyModeState(keyboardHost, chinese)
                // **必须重建键盘**：三张有标点的页（字母/数字/符号）的上滑内容都
                // 按中英模式各生成一份，键帽小字是画进 Key 里的。不重建的话模式切了
                // 但键帽还显示中文标点、上滑仍打出「。，？！」——英文模式邮箱都写不完。
                showPage(page)
                // 切英文时把中文候选丢掉：那批候选是按中文码查的，留着只会误导
                forgetContext()
                // **必须显式清空组合串和候选。**
                //
                // 用户实测的场景：中文打了一串拼音 → 点中英切到英文 → 敲空格，
                // 结果上屏的还是中文。原因是 forgetContext() 只清联想
                // （prevWord + suggestions），`pending`（那串拼音）和 `candidates`
                // 原封不动；空格分支里 `chinese` 已经是 false，于是走到
                // 「commitEnglishComposing() 用 pending 确认原文」那条路，
                // 把中文拼音当英文字母串直接上屏了。
                //
                // 语义上这是对的：切到英文态的那一刻，用户放弃了那串拼音的转换。
                // 留着一个既不是中文候选、也不能继续按拼音编辑的死字符串，
                // 只会让下一个按键产生意外结果（空格、回车、退格三条路都受影响）。
                pending.setLength(0)
                code = ""
                candidates = emptyList()
                pendingSegments = emptyList()
                segmentState = null
                segmentStateCode = ""
                englishWords = emptyList()
                englishSuggest?.query("")
                // 组合区也要撤掉：宿主里那串带下划线的拼音必须消失，
                // 否则切英文后输入框里还挂着上一条中文的未确认字符。
                currentInputConnection?.setComposingText("", 1)
                // 快捷输入只在中文态存在；切到英文时手上那截 `/rq` 不能留在组合态里
                if (quickMode) {
                    quickMode = false
                    quickWords = emptyList()
                }
                refreshCandidates()
            }

            KeyKind.SELECT -> {
                // 分段态下数字行是隐藏的，这里再挡一道：万一残影被点到，
                // 也不能拿"第 N 个候选"去解释一个分段 chip 的下标。
                if (segmentActive) return
                // 数字选词：code 是 "1".."9"，越界（候选没那么多）直接忽略。
                // 下标换算在 NumberSelect.indexOf 里，那边钉死了 -1。
                val digit = key.code.toIntOrNull() ?: return
                if (quickMode) {
                    val index = NumberSelect.indexOf(digit, quickWords.size) ?: return
                    commitQuick(quickWords[index])
                    return
                }
                val window = visibleCandidates()
                val index = NumberSelect.indexOf(digit, window.size) ?: return
                commitWord(window[index])
            }

            KeyKind.PAGE -> showPage(
                when (key.code) {
                    "letters" -> Page.LETTERS
                    "symbols" -> Page.SYMBOLS
                    "emoji" -> Page.EMOJI
                    "nav" -> Page.NAV
                    else -> Page.DIGITS
                },
            )

            KeyKind.TEXT -> {
                if (quickMode) {
                    if (quickWords.isNotEmpty()) commitQuick(quickWords.first()) else commitPending()
                    commitText(key.code)
                    forgetContext()
                    return
                }
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
                // 双击 = 锁定/解锁，单击 = 一次性。判定窗口和迁移都在 ShiftMachine 里。
                val now = System.currentTimeMillis()
                val (next, double) = ShiftMachine.onTapWithTime(shiftState, lastShiftTapAt, now)
                // 双击之后把时间戳清零：再点第三下必须是「新的一次单击」，
                // 否则连点三下会一直判成双击来回翻。
                lastShiftTapAt = if (double) 0L else now
                shiftState = next
                // 只改这一颗键的配色/标签，不重建键盘（重建会闪、候选条会抖）
                KeyboardView.applyShiftState(keyboardHost, shiftState)
            }

            KeyKind.NAV -> {
                // 光标/编辑控制。先把手上的组合串收尾，否则移动光标时那串拼音
                // 还挂在输入框里，宿主会把它当成一次新的编辑。
                if (!commitSegmentIfActive()) commitPending()
                NavAction.of(key.code)?.let { performNav(it) }
            }
        }
    }

    /** 长按回调。⇧ 切锁定态；空格键长按说话（松手上屏）。⌫ 的连删在 KeyboardView 内部消化了。 */
    private fun onLongKey(key: Key) {
        if (key.kind == KeyKind.SPACE) {
            // 长按空格 = 按住说话。这是「手里已经在打字，突然想说一句」的最短路径：
            // 不用抬手去够状态条上的麦克风。
            //
            // 只处理「开始」；结束靠松开时 KeyboardView 发的 ACTION_UP（见下面 pump）。
            // 正在录音或者已有语音占着槽位时忽略，避免两个录音线程抢 AudioRecord。
            if (voiceState == VoiceRecorder.State.RECORDING ||
                voiceState == VoiceRecorder.State.LOADING ||
                voiceState == VoiceRecorder.State.RECOGNIZING
            ) {
                return
            }
            if (!voiceEnabled) return
            voiceWave?.resume()
            voice?.start()
            return
        }
        if (key.kind != KeyKind.SHIFT) return
        shiftState = ShiftMachine.onLongPress(shiftState)
        KeyboardView.applyShiftState(keyboardHost, shiftState)
    }

    /**
     * 执行一个光标/编辑动作。
     *
     * ## 方向键为什么要两条路都走
     *
     * 标准做法是 `sendKeyEvent(KeyEvent(KEYCODE_DPAD_LEFT))`，绝大多数原生输入框都认。
     * 但 WebView / Electron 里的 `<input>`、部分 Compose 文本框对 DPAD 事件不响应——
     * 用户看到的就是「按左键没反应」。
     *
     * 所以：先试 `setSelection`（直接改光标位置，最可靠），失败再退回 `sendKeyEvent`。
     * `setSelection` 需要先读到当前选区，读不到（宿主不给 getExtractedText）就只能发键。
     *
     * ## 为什么复制粘贴走 performContextMenuAction
     *
     * `commitText(剪贴板内容)` 是最常见的错误写法：它会把内容**插在光标处**而不是
     * 替换选区，而且完全不经过宿主的编辑历史。`performContextMenuAction(android.R.id.copy)`
     * 才是输入法该用的正规入口——宿主自己决定复制什么、是否允许复制（密码框会拒绝）。
     */
    private fun performNav(action: NavAction) {
        val ic = currentInputConnection ?: return
        when (action) {
            NavAction.LEFT, NavAction.RIGHT, NavAction.UP, NavAction.DOWN ->
                moveCursor(ic, action)
            NavAction.SELECT_ALL -> ic.performContextMenuAction(android.R.id.selectAll)
            NavAction.COPY -> ic.performContextMenuAction(android.R.id.copy)
            NavAction.PASTE -> ic.performContextMenuAction(android.R.id.paste)
            NavAction.CUT -> ic.performContextMenuAction(android.R.id.cut)
            NavAction.LINE_START -> jumpToLineEdge(ic, toStart = true)
            NavAction.LINE_END -> jumpToLineEdge(ic, toStart = false)
        }
        // 光标动了，之前缓存的「上一个词」已经不对应新位置，联想必须重算，
        // 否则会在新光标处推荐上一个位置的词。
        forgetContext()
        refreshCandidates()
    }

    /**
     * 移动光标。先用 setSelection 直改，读不到选区才发 DPAD 键。
     *
     * 上下两行在单行输入框里没有意义，此时 `getTextBeforeCursor` 里没有 `\n`，
     * 所以按「没有换行就直接忽略」处理——比乱跳到文本开头好。
     */
    private fun moveCursor(ic: InputConnection, action: NavAction) {
        val extracted = ic.getExtractedText(ExtractedTextRequest(), 0)
        val selStart = extracted?.selectionStart ?: -1
        val selEnd = extracted?.selectionEnd ?: -1
        if (selStart < 0 || selEnd < 0) {
            // 宿主不支持 getExtractedText：退回发键，让宿主自己处理
            val keyCode = when (action) {
                NavAction.LEFT -> KeyEvent.KEYCODE_DPAD_LEFT
                NavAction.RIGHT -> KeyEvent.KEYCODE_DPAD_RIGHT
                NavAction.UP -> KeyEvent.KEYCODE_DPAD_UP
                else -> KeyEvent.KEYCODE_DPAD_DOWN
            }
            ic.sendKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, keyCode))
            ic.sendKeyEvent(KeyEvent(KeyEvent.ACTION_UP, keyCode))
            return
        }
        // 纯计算全在 NavCursor 里（有 JVM 单测）：这里只负责把结果塞回宿主。
        val text = extracted!!.text ?: return
        val target = NavCursor.move(text, selEnd, action) ?: return
        ic.setSelection(target, target)
    }

    /** 移到行首 / 行尾。单行输入框里就等于文本开头 / 结尾。 */
    private fun jumpToLineEdge(ic: InputConnection, toStart: Boolean) {
        val extracted = ic.getExtractedText(ExtractedTextRequest(), 0)
        val text = extracted?.text ?: return
        val pos = extracted.selectionEnd
        if (pos < 0) return
        val target = if (toStart) NavCursor.lineStart(text, pos) else NavCursor.lineEnd(text, pos)
        ic.setSelection(target, target)
    }

    /** 把组合串同步进输入框（带下划线），然后刷新候选。 */
    private fun pushComposing() {
        val ic = currentInputConnection ?: return
        ic.setComposingText(pending.toString(), 1)
        refreshCandidates()
        refreshStatus()
    }

    /**
     * 英文输入：逐字**直接上屏**，不再用组合态。
     *
     * ## 为什么从 setComposingText 改成 commitText
     *
     * 组合态（`setComposingText`）本该让宿主机把未确认的文字画成带下划线的样子。
     * 但**不是每个宿主都画**：Termux 这类终端视图只处理真正提交的文本，
     * 组合区对它来说是不存在的——用户在 Termux 里打英文**一个字都看不见**，
     * 按一下空格才突然冒出来一整串。这是实际反馈的场景。
     *
     * 拼音没这个问题（用户说的是「英文模式在 Termux 命令行看不到」），
     * 因为拼音本来就只显示在候选条上，不进输入框。
     *
     * 换成直接上屏的代价：
     * - 失去「点建议词自动替换组合区」的能力 → 改成 [pickEnglishWord] 里自己
     *   回删 `pending.length` 个字符再写新词。
     * - 输入框里立即出现最终文本，没有「未确认」这个中间态。
     *   对输入法来说这是更安全的行为：用户看到的永远是真实上屏内容。
     *
     * `pending` 仍然保留，它是「我刚打进宿主的那串字母」的账本，退格和建议替换
     * 都靠它的长度定位。
     */
    private fun pushComposingEnglish() {
        val ic = currentInputConnection ?: return
        // 只把**最新一个字母**提交上去：调用方每敲一个键追加一个字符，
        // 这里增量提交，避免每次都重写整串（重写会多一次光标移动，
        // 在终端里表现为闪烁）。
        val text = pending.toString()
        if (text.isEmpty()) return
        ic.commitText(text.takeLast(1), 1)
        refreshStatus()
        // 建议是异步的（跨进程问系统拼写检查器），回来再渲染
        englishSuggest?.query(text)
    }

    /** 英文建议回来了：存起来并只重画候选条，不动输入框。 */
    private fun onEnglishSuggest(prefix: String, words: List<String>) {
        // 用户在等待期间又改了词 → 这批结果作废
        if (!chinese && prefix != pending.toString()) return
        englishWords = words
        renderCandidates()
    }

    /**
     * 结束英文输入：清掉账本。
     *
     * ## 为什么不再 commitText
     *
     * 英文改成逐字直接上屏之后（见 [pushComposingEnglish]），字母**已经在宿主里**了。
     * 这里如果还 `commitText(pending.toString())`，用户会看到每个词都变成
     * `hellohello` ——把同一串又打了一遍。
     *
     * 空格和回车都走这里，它们的职责只剩「结束这个词」：清账本、收起候选条。
     *
     * @return 是否确实结束了一次英文输入（空串返回 false，让调用方继续走原逻辑）
     */
    private fun commitEnglishComposing(): Boolean {
        if (chinese || pending.isEmpty()) return false
        pending.setLength(0)
        englishWords = emptyList()
        englishSuggest?.query("")
        refreshCandidates()
        refreshStatus()
        forgetContext()
        return true
    }

    /** 形码最多几键：自然码 1、小鹤 2、关闭 0。和 core 的 `AssistMode::max_keys` 对齐。 */
    private fun assistMaxKeys(): Int = when (assistMode) {
        "ziranma" -> 1
        "xiaohe" -> 2
        else -> 0
    }

    /**
     * 现在该不该出现「形」按钮。
     *
     * 只有「中文态 + 正在打字（或展开了某一段）+ 方案没关」这一种情况。英文态、快捷
     * 输入、刚上屏只剩联想的时候都没有可筛的码，不出现。
     */
    private fun assistAvailable(): Boolean =
        assistMaxKeys() > 0 && chinese && !quickMode &&
            (pending.isNotEmpty() || (segmentState?.focus ?: -1) >= 0)

    /**
     * 退出形码态并清掉已打的形码。
     *
     * ## 为什么这里要重建键盘
     *
     * 形码态的按键是「字母小字 + 部件提示大字」的双层 FrameLayout，跟普通字母键
     * **不是同一个 View 结构**。只把 [assistActive] 置 false，键帽不会自己变回去——
     * 用户原话：「形码开启后 选了确定下来的字后 键盘上的形码还在 应该要自动收掉，
     * 回归之前字母和标点符号的键盘布局」。
     *
     * 收敛到这一个函数，是因为退出形码态有八条路径：选词上屏（commitWord）、联想词、
     * 原样上屏、切中英、英文态早退、回 chip 视图、分段选完……散落着补「重建」必漏。
     * 判据见 [AssistExit.shouldRebuildKeyboard]：**只在 true→false 这一次跳变**重建，
     * 普通选词（本来就 false）不重建，避免每选一个词都抖一下。
     */
    private fun resetAssist() {
        val wasActive = assistActive
        assistActive = false
        assistKeys.setLength(0)
        assistWords = emptyList()
        if (AssistExit.shouldRebuildKeyboard(wasActive)) {
            rebuildKeyboard()
        }
    }

    /** 点「形」：进/出形码态。进出都清空已打的形码——上一次的筛选不该带过来。 */
    private fun toggleAssist() {
        if (!assistActive && !assistAvailable()) return
        assistActive = !assistActive
        assistKeys.setLength(0)
        assistWords = emptyList()
        // **必须重建键盘**：形码态要把按键换成「字母小字 + 形码大字」，
        // 那是另一个 View 结构（FrameLayout 双层），不是就地改字能变的。
        //
        // 只在切页/切模式时才重建键盘的老习惯在这里不适用——那个「不重建」
        // 是为了防止候选条抖动，而进出形码态本来就是用户主动的一次模式切换，
        // 抖一下是可接受的，看不到提示才是真问题。
        rebuildKeyboard()
        refreshAssist()
    }

    /** 重建当前键盘页（只换键盘区，不动候选条/InputView）。 */
    private fun rebuildKeyboard() {
        val host = keyboardHost ?: return
        host.removeAllViews()
        showPage(page)
    }

    /**
     * 形码变了之后重算候选。
     *
     * 分段态**不能**走 [refreshCandidates]：那条路会 `refreshSegmentMode()` 重建整个
     * SegmentState，用户刚展开的那一段和已选的词全被冲掉。所以这里分开处理。
     */
    private fun refreshAssist() {
        if (segmentState != null) {
            updateAssistWords()
            renderCandidates()
            return
        }
        if (!assistActive || assistKeys.isEmpty()) {
            refreshCandidates()
            return
        }
        candidates = NativeEngine.lookupAssist(
            pending.toString(),
            pinyinMode,
            assistKeys.toString(),
        )
        // 形码一变，候选整批换掉：回到第一页，展开态的懒加载也要从头来
        candPage = 0
        expandedLoaded = 0
        renderCandidates()
    }

    /** 重算当前展开段的筛选结果。段码来自 `pendingSegments`（和 segments 同序）。 */
    private fun updateAssistWords() {
        assistWords = emptyList()
        if (!assistActive || assistKeys.isEmpty()) return
        val st = segmentState ?: return
        if (st.focus < 0) return
        val code = pendingSegments.getOrNull(st.focus)?.code ?: return
        assistWords = NativeEngine.lookupAssist(code, pinyinMode, assistKeys.toString())
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
        if (quickMode) {
            // 快捷输入和拼音是两套查询。**这里绝不能再查词库**：`/rq` 会被当成
            // 双拼码，然后候选条上冒出一排毫不相干的字，用户根本没机会看到日期。
            code = ""
            candidates = emptyList()
            pendingSegments = emptyList()
            segmentState = null
            segmentStateCode = ""
            // 只打了 `/` 时 key 是空串，引擎返回全部命令的当前值——正好当预览。
            quickWords = NativeEngine.quickCandidates(
                pending.toString().removePrefix(NativeEngine.quickPrefix()),
                QUICK_CAND_LIMIT,
            )
            renderCandidates()
            return
        }
        if (!chinese || pending.isEmpty()) {
            code = ""
            candidates = emptyList()
            pendingSegments = emptyList()
        } else {
            // 一次多要一点：候选条按页显示（每页 = 设置里的「候选个数」），翻页
            // 需要后面的候选已经在手上——每翻一页都重查一遍词库是白费。45 个够翻五页。
            val lookup = NativeEngine.lookupInput(pending.toString(), pinyinMode, CAND_FETCH)
            code = lookup.code
            candidates = lookup.words
            pendingSegments = lookup.segments
            // 新的一次查询 = 从第一页开始看，展开态懒加载从头加载
            //
            // **expandedLoaded 必须归零**：它是「已渲染多少个」的游标。不归零的话，
            // 用户先在 l（565 个）滑到底加载了 300 个，再按退格变成 li（281 个），
            // 游标还停在 300 —— subList(0, 281) 会越界，或者新候选一展开就是
            // 一大片，把懒加载的意义完全抵消。
            candPage = 0
            expandedLoaded = 0
            // **横向滚动位置也要归零。**
            //
            // 单行候选条是 HorizontalScrollView，用户滑到右边挑完一个字之后，
            // 下一个字的候选条还停在那个 x 偏移——新候选的前几个被推出视野，
            // 看起来像「候选没了」。用户报的就是这个。
            //
            // 展开态不用管：那时候 `candidateScroll` 是 GONE，候选挂在
            // `expandedScroll`（纵向）上，没有横向偏移这回事。
            // post 一帧：刚换完候选时子 View 还没量宽度，同步 scrollTo 会被
            // 后续的 layout 覆盖回旧值。
            candidateScroll?.post { candidateScroll?.scrollTo(0, 0) }
        }
        refreshSegmentMode()
        renderCandidates()
    }

    /**
     * 换段/选中之后统一收尾：清形码、同步候选、候选条横向归零。
     *
     * 抽出来是因为「选了字 → 清形码 → 刷候选 → 复位横滑」这套动作在
     * 展开分段、单行分段、形码筛选三处一模一样，复制三份必然漏其中一处
     * （横向位置残留就是这么漏出来的）。
     */
    private fun afterSegmentChosen(next: com.feihe.ime.segment.SegmentState) {
        segmentState = next
        // 记下这串码：后续重绘（展开/收起、窗口变化）靠它判断「码没变，别丢分段态」
        segmentStateCode = pending.toString()
        // 换段清形码：留着会把下一段的候选筛空
        assistKeys.setLength(0)
        if (next.focus < 0 && assistActive) {
            // **选完字自动退出形码态**（用户要求：选择了字之后退出这个模式）。
            // 走 resetAssist()：统一负责清状态 + 重建键盘回字母/标点布局，
            // 别在这里手写 assistActive=false + rebuildKeyboard（会和上屏路径漂）。
            resetAssist()
        }
        updateAssistWords()
        // **最后一段选完 = 整句齐了，直接上屏。**
        // 否则用户选完最后一个字还得再按空格，而候选条上已经没有任何提示了。
        if (next.focus < 0) {
            commitSegmentIfActive()
        } else {
            // 换到下一段 = 全新的候选列表，横向位置必须归零。
            // 用户滑到右边挑完这个字，下一个字的候选如果还停在那个偏移，
            // 前面的候选就被推出视野，看起来像「没候选了」。
            candidateScroll?.post { candidateScroll?.scrollTo(0, 0) }
            renderCandidates()
        }
    }

    /**
     * 要不要进「逐段选择」态。
     *
     * 判据**完全来自引擎**：`pendingSegments` 为空就是普通候选——它是这次查询里
     * core 顺手算好带回来的（`lookup_input.segments`），不是壳猜的。
     */
    /**
     * 要不要进「逐段选择」态。
     *
     * ## 不再自动进（这是「退格不进选字模式」的根因）
     *
     * 以前这里是：
     *
     * ```kotlin
     * segmentState = SegmentState(...).takeIf { it.active }
     * ```
     *
     * 只要引擎顺手带回了分段数据就**立刻进分段态**。`hexi` 这种短码一旦整码查不到、
     * 走了整句兜底，引擎就会返回 segments——于是用户打完字，候选条已经是 chip，
     * 而不是他想要的候选词条。
     *
     * 后果连锁：`backspaceIntoSegments()` 第一句是 `if (segmentState != null) return false`
     * （那是给「已经分段、退格删字母」用的），所以退格直接落到删字母——
     * 用户按退格想进选字模式，结果吃掉一个字母。他报的正是这个。
     *
     * 现在改成**只由退格触发**：分段数据继续搭车回来、存在 `pendingSegments` 里
     * 备用，但界面停在普通候选条。候选条第一项就是整句（引擎拼好的），
     * 用户想整句上屏直接点第一项或按空格，想换字再按退格。
     *
     * 想恢复「自动进分段」，把这里改回 takeIf 即可——但先想清楚退格那条分支
     * 怎么同时支持「进分段」和「删字母」两种意图。
     */
    private fun refreshSegmentMode() {
        // 候选变了就把旧的分段态收掉：留着会指向上一串码的段，选出来的字对不上。
        // 退格触发的新分段态由 backspaceIntoSegments() 自己设，不走这里。
        //
        // **但"候选变了"必须以「那串码真的变了」为准，不能靠"调了 refresh"来判断。**
        //
        // 这里原来是无条件 `if (segmentState != null) segmentState = null`，
        // 造成一个很隐蔽的 bug（用户报：组词时选最后一个字，点展开候选，
        // 前面选的全没了、直接上屏字母串）：
        //
        //   展开候选 → applyExpandedChrome(true) 把键盘设成 GONE
        //            → 宿主重新布局，重入 onStartInputView()
        //            → 它调 refreshCandidates() → 这里清掉 segmentState
        //            → 已选的字全丢，候选条退回普通模式的字母串
        //
        // 换句话说：**同一个 pending 重新渲染一次就自毁**。展开/收起、
        // 窗口尺寸变化、宿主重发 onStartInputView 都会踩到。
        //
        // 正确判据：分段态记录它是在哪串码上建立的（segmentStateCode）。
        // 码没变 → 这是重绘，分段态必须原样保留；码变了 → 才该丢弃。
        if (segmentState != null && segmentStateCode != pending.toString()) {
            segmentState = null
            segmentStateCode = ""
        }
    }

    /**
     * 分段态下按了收尾键（空格/回车/标点/字符）：先把整句上屏。
     *
     * **这里必须学一次词**：逐字选词是用户造词的主要路径（打 feih → 选「飞」→
     * 选「和」），而它全程不经过 [commitWord]，所以以前这条路一个字都没记住。
     * 用户体感就是「自组词功能依然不生效」。
     */
    /**
     * 分段态下按空格的语义：**确认当前段 → 前进下一段**；没有下一段时整句上屏。
     *
     * 等价于「点一下当前高亮的那个候选」，只是不用手指去点。
     *
     * @return 是否处理了（false = 当前不在分段态，调用方继续走普通空格逻辑）
     */
    private fun commitSegmentStepOrFinish(): Boolean {
        val state = segmentState ?: return false
        // chip 视图（focus = -1）：没有"当前段"可确认，空格就是整句上屏
        if (state.focus !in state.segments.indices) {
            return commitSegmentIfActive()
        }
        // 形码筛过时界面显示的是子集，必须按**词**选，不能用下标
        // （下标是子集里的位置，和 segments[focus] 对不上）
        val next = if (assistActive) {
            val word = state.wordAt(state.focus)
            if (word.isEmpty()) return commitSegmentIfActive()
            state.chooseWord(word, keepOpen = true)
        } else {
            // 未筛选：直接确认当前选中的那一个（不动下标，只推进 focus）
            state.choose(state.selected[state.focus])
        }
        afterSegmentChosen(next)
        return true
    }

    private fun commitSegmentIfActive(): Boolean {
        val state = segmentState ?: return false
        val sentence = state.sentence()
        commitWord(sentence, thenSuggest = false)
        rememberPhrase(sentence, state)
        return true
    }

    /**
     * 把逐段拼出来的整句记成自造词。
     *
     * ## 这里踩过两次坑，都值得记下来
     *
     * **第一次**：以为「逐字选词不经过 `commitWord`」是根因，补了 `learn(码, 整句)`
     * ——但因为码传的是双拼码 `fwhe`，而 `phrase::learn` 里第一句就是
     * `encode_pinyin(Xiaohe, pinyin).is_none()` 校验**全拼**，双拼码过不了，
     * 静默 return，等于什么都没做。用户反馈「组过一次词下次还是没有」。
     *
     * **第二次**：想用 `dict::pinyin_of(code, word)` 把双拼码反查成全拼，但那函数
     * **只在词库已有该词时**才返回拼音。用户刚造的新词当然不在词库里 → 恒为空串
     * → 又走「拿不到全拼就放弃」那条路 → 还是不学。
     *
     * 真正的解法是不依赖词库：用 `shuangpin::decode_pinyin` 把码直接解成全拼
     * （`fwhe` → `fei he`，纯查表，不碰词库）。
     */
    private fun rememberPhrase(sentence: String, state: SegmentState) {
        val code = state.sentenceCode()
        val full = if (code.isEmpty()) "" else NativeEngine.decodePinyin(code)
        // 单字不记成词：一个人名/术语至少要两个字才有"一次输入上屏"的价值
        if (sentence.length < 2) return
        // 码数必须和段数一致，否则解出来的全拼和词不对应
        if (code.isEmpty()) return
        // 解不出来（畸形码）就放弃：宁可这次不学，也别往 phrases.txt 写一条
        // 谁也查不出来的死数据。
        if (full.isEmpty()) return
        NativeEngine.learnPhrase(sentence, full)
    }

    private fun renderCandidates() {
        val col = candidateColumn ?: return
        renderNumberRow()
        // **英文态：只画单词建议，不画中文候选。**
        //
        // 参照豆包：英文输入时候选栏给单词补全（hello/help…），点一下替换掉
        // 输入框里未确认的那串。没有建议时整个候选条隐藏，键盘上不留空条。
        //
        // 放在最前面而不是在末尾清空：后面几十行绑定逻辑全部跳过，
        // 也杜绝了「某条分支忘了检查 chinese」再次漏出中文候选的可能。
        if (!chinese && !quickMode) {
            resetAssist()
            suggestions = emptyList()
            val words = englishWords
            if (words.isEmpty()) {
                candidateScroll?.visibility = View.GONE
                expandedScroll?.visibility = View.GONE
                expandedColumn?.removeAllViews()
                updateTopSlot()
                return
            }
            expandedScroll?.visibility = View.GONE
            expandedColumn?.removeAllViews()
            candidateScroll?.visibility = View.VISIBLE
            val bar = ensureSingleRow(col)
            while (bar.childCount > words.size) bar.removeViewAt(bar.childCount - 1)
            while (bar.childCount < words.size) bar.addView(TextView(this))
            words.forEachIndexed { idx, word ->
                val v = bar.getChildAt(idx) as TextView
                bindCandidateView(v, word, idx == 0)
                // 重绑点击：bindCandidateView 里绑的是中文 commitWord 那条路，
                // 英文必须走「替换组合区」，否则会把建议词追加在原词后面。
                v.setOnClickListener { pickEnglishWord(word) }
            }
            updateTopSlot()
            return
        }
        if (quickMode) {
            // 快捷结果同样是「点一下就上屏」，复用候选项的画法；但**不显示联想**
            // ——`/rq` 出来的日期后面接什么都对，联想只会把结果挤到看不见。
            val bar = ensureSingleRow(col)
            val total = quickWords.size
            while (bar.childCount > total) bar.removeViewAt(bar.childCount - 1)
            while (bar.childCount < total) bar.addView(TextView(this))
            quickWords.forEachIndexed { idx, word ->
                bindCandidateView(bar.getChildAt(idx) as TextView, word, idx == 0)
            }
            updateTopSlot()
            return
        }
        val seg = segmentState
        if (seg != null) {
            // 分段态优先用**多行展开**：用户在这个态里逐字挑词，单行横向滚着找字
            // 是最难受的（一屏只能看六七个，还得滑）。展开态把键盘让出来，
            // 整屏都是候选，找字快得多。
            //
            // chip 视图（focus = -1）仍然走单行：一行 chip 是「整句长什么样」的
            // 预览，铺成多行反而看不出分词边界。
            if (candidateExpanded && seg.focus >= 0) {
                val exp = expandedColumn ?: return
                // **必须显式调 applyExpandedChrome(true)**：它负责
                // ① 隐藏键盘 ② 显示底部「▴ 收起候选」条。
                //
                // 漏了这句的后果就是用户报的「展开之后收不回去」：
                // 键盘已经被藏掉了（候选区占满屏），而收起条是 GONE 的，
                // 屏幕上没有任何一个地方能点回单行——除了再按一次展开键，
                // 可那时展开键也随键盘一起没了。
                applyExpandedChrome(true)
                candidateScroll?.visibility = View.GONE
                expandedScroll?.visibility = View.VISIBLE
                renderExpandedSegmentRows(exp, seg)
                updateTopSlot()
                return
            }
            // 分段态的 chip 带选中底色、还有「‹ 返回」这种特殊项，画法不同且低频，
            // 直接整条重画（它不占键盘高度，重建的抖动没人看得见）。
            //
            // **必须调 applyExpandedChrome(false)**：用户在展开态选了字之后
            // `focus` 会前进到下一段，这时如果还是展开态就走上面那条分支；
            // 但走到这里（focus = -1，chip 视图）说明该收键盘了。漏了这句的话
            // 键盘会一直藏着，用户从展开态退回 chip 后看不见键盘。
            applyExpandedChrome(false)
            val bar = ensureSingleRow(col)
            bar.removeAllViews()
            renderSegmentBar(bar, seg)
            updateTopSlot()
            return
        }
        // 验证码提示只在**没有候选**时占第一格：一旦开始打字它就隐身。
        val otp = otpCode.takeIf { candidates.isEmpty() && pending.isEmpty() }

        // 「形」按钮：有码可筛时占第一格。它不参与数字选词（下标从候选开始算）。
        val assistChip = assistAvailable()
        val window = visibleCandidates()
        val hasPrev = candPage > 0
        val hasNext = CandidateGrid.hasNext(
            candidates.size, candPage, candidateExpanded, pageSize(), expandedRows(),
        )
        // 形码筛空了：给一句提示。否则候选条上只剩一个「形」按钮，看着像坏了。
        val emptyAssist = assistActive && assistKeys.isNotEmpty() && window.isEmpty()

        // 展开态：把键盘整块藏起来，候选区吃掉腾出的所有高度（能铺七八行）。
        // 收起态：键盘还原、候选条回到单行。
        applyExpandedChrome(candidateExpanded)

        // 展开态：多行 × 每行 6 个。收起态：单行横滑。
        //
        // 两条路用**不同的父容器**：展开态挂 expandedColumn（普通 LinearLayout，
        // 宽度跟着屏幕量），收起态挂 candidateScroll（HorizontalScrollView）。
        // 不能共用一个——HorizontalScrollView 给子 View 无限宽度约束，多行时
        // 行内 MATCH_PARENT 退化成 WRAP_CONTENT、weight 量成 0，候选会塌成一片。
        //
        // **不拿 assistChip / 分段态当展开的前提**：以前这里写了 `&& !assistChip`，
        // 而 assistAvailable() 在「正在打拼音」时恒为 true——用户打着字点展开，
        // 条件不成立，函数直接走到收起分支，界面毫无变化。用户看到的就是
        // 「展开没有多行」。展开是用户明确点的动作，优先级高于任何自动态。
        if (candidateExpanded) {
            // **兜底：手上没有任何可画的东西时，展开态没有存在意义。**
            //
            // 展开态是把键盘整块藏掉、借它的高度来铺候选。一旦候选和分段态都被
            // 清空（上屏、删光字母、形码筛空），继续撑着就是一块空白面板 +
            // 一根没人点的「收起」条——用户还得手动点一次才能拿回键盘。
            //
            // 正常路径上 commitWord / commitQuick 已经主动收了展开态（根因修在
            // 那里）。这层兜底是防**将来新增的上屏/清空路径**再次漏掉：
            // 收起态最坏只是候选条变窄，留着展开态却是键盘消失 + 空白面板。
            val segNow = segmentState
            val keep = com.feihe.ime.keyboard.ExpandedGate.keepsExpanded(
                windowSize = window.size,
                segmentFocus = segNow?.focus ?: -1,
                segmentCount = segNow?.segments?.size ?: 0,
            )
            if (!keep) {
                candidateExpanded = false
            } else {
                val exp = expandedColumn ?: return
                candidateScroll?.visibility = View.GONE
                // 可见性改在 ScrollView 上：expandedColumn 是它的内容，
                // 让内容可见而容器 GONE 等于什么都不显示。
                expandedScroll?.visibility = View.VISIBLE
                renderExpandedRows(exp, window)
                updateTopSlot()
                return
            }
        }
        // 收起态：把展开容器清空藏起，避免它继续占位或残留上一次的内容
        expandedScroll?.let { sv ->
            sv.visibility = View.GONE
            // 高度还回一行，并把滚动位置归零——下次展开从第一行看起
            (sv.layoutParams as? LinearLayout.LayoutParams)?.let { lp ->
                if (lp.height != dp(CANDIDATE_ROW_DP) || lp.weight != 0f) {
                    lp.height = dp(CANDIDATE_ROW_DP)
                    lp.weight = 0f
                    sv.layoutParams = lp
                }
            }
            sv.scrollTo(0, 0)
        }
        expandedColumn?.removeAllViews()
        candidateScroll?.visibility = View.VISIBLE
        val bar = ensureSingleRow(col)

        // 复用已有的 TextView：候选 + 分隔符 + 联想按顺序对位，多了删、少了补。
        // 格子总数交给 CandidateGrid.barSlots 算，和单测共用同一份逻辑：
        // 之前这里手写条件链漏了「展开/收起箭头」，候选从 1 个变成 2 个以上时
        // 多绑一格而容器没扩容，getChildAt 返回 null，`as TextView` 直接 NPE 闪退
        // （现象就是按第一个键正常、按第二个键必崩）。
        val expandToggle = candidates.size > 1
        // **候选条靠左再放一个 😊 表情入口（在「形」和验证码之后）。**
        //
        // 为什么必须在这里：字母页底排的 😊 键已按要求搬到状态条，但状态条在
        // 「正在打字/有候选」时是 GONE 的（TOP_SLOT_DP 那个槽位让给了候选）。
        // 只放状态条的话，打字打到一半想插个表情就没入口了——那正是最常用的时刻。
        // 两者是同一个动作（showPage(EMOJI)），不冲突。
        //
        // **但不能排在「形」前面**：形码筛选态和验证码提示是状态位，压在最左是既有
        // 约定，表情挤过去会让人以为筛选按钮消失了（见下面绑定顺序的注释）。
        val emojiChip = true
        val total = CandidateGrid.barSlots(
            assistChip = assistChip,
            otp = otp != null,
            hasPrev = hasPrev,
            candidateCount = window.size,
            hasNext = hasNext,
            emptyAssist = emptyAssist,
            expandToggle = expandToggle,
            suggestionCount = suggestions.size,
            emailSuffixCount = emailSuffixes.size,
            emojiChip = emojiChip,
        )
        while (bar.childCount > total) bar.removeViewAt(bar.childCount - 1)
        while (bar.childCount < total) bar.addView(TextView(this))
        // 兜底：候选条里少一格只是少显示一个候选，绝不能让按键崩掉整个输入法。
        // 这个 total 是由一串条件拼出来的，将来再加一种格子很容易又漏算。
        var i = 0
        fun nextSlot(): TextView? = bar.getChildAt(i++) as? TextView
        // 展开/收起箭头放**第一位**，紧跟在「形」和验证码之后。
        //
        // 以前放在最末尾：候选一多（比如 45 个），它就排到要横滑到最右边才看得见，
        // 等于把入口藏起来了。收起态候选条是横滑的，左边永远默认可见，放这才找得到。
        //
        // **绑定顺序：形码筛选 → 验证码 → 表情 → 展开箭头 → 候选…**
        //
        // 表情入口排在 assistChip/otp 之后：那两个是「形码筛选」「验证码提示」这种
        // **状态位**，必须压在最左，表情挤到它们前面会让人以为筛选按钮没了。
        if (assistChip) nextSlot()?.let { bindAssistView(it) }
        if (otp != null) nextSlot()?.let { bindOtpView(it, otp) }
        if (emojiChip) nextSlot()?.let { bindEmojiChip(it) }
        if (expandToggle) nextSlot()?.let { bindExpandToggle(it) }
        if (hasPrev) nextSlot()?.let { bindPageView(it, "‹", -1) }
        if (emptyAssist) nextSlot()?.let { bindHintView(it, "形码无匹配，退格重试") }
        window.forEachIndexed { idx, word ->
            nextSlot()?.let { bindCandidateView(it, word, idx == 0, code) }
        }
        if (hasNext) nextSlot()?.let { bindPageView(it, "›", 1) }
        if (suggestions.isNotEmpty()) {
            nextSlot()?.let { bindSeparatorView(it) }
            suggestions.forEach { word ->
                nextSlot()?.let { bindSuggestionView(it, word) }
            }
        }
        // 邮箱后缀候选排在最后：它和词库联想都是「点一下补全」，但来源不同，
        // 中间用同一条分隔线隔开（bindSeparatorView 已经在上面用掉了，
        // 这里再借一格会多出一道竖线，所以直接接在后面靠视觉间距区分）。
        if (emailSuffixes.isNotEmpty()) {
            emailSuffixes.forEach { suffix ->
                nextSlot()?.let { bindEmailSuffixView(it, suffix) }
            }
        }
        updateTopSlot()
    }

    /**
     * 保证 [candidateColumn] 里只有一行，并返回那一行。
     *
     * 收起态 / 快捷输入 / 分段态都走这里：它们都是「一行横排」。展开态切换回来时
     * 要把多余的行删掉，否则候选条会留在 3 行高的状态而内容只有 1 行。
     */
    private fun ensureSingleRow(col: LinearLayout): LinearLayout {
        while (col.childCount > 1) col.removeViewAt(col.childCount - 1)
        val bar = (col.getChildAt(0) as? LinearLayout) ?: LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(dp(8f), 0, dp(8f), 0)
            col.addView(
                this,
                LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.WRAP_CONTENT,
                    dp(CANDIDATE_ROW_DP),
                ),
            )
        }
        candidateBar = bar
        resizeCandidateHost(1)
        return bar
    }

    /**
     * 展开态：按 [CandidateGrid.PER_ROW] 切成多行，逐行铺候选。
     *
     * 不加数字编号——触屏上直接用手指点，编号是给键盘选词的电脑用的。
     */
    /**
     * 展开态渲染「当前段的候选」，多行铺开。
     *
     * 和 [renderExpandedRows] 的区别只有绑定逻辑：这里的词属于**同一段**，
     * 点一下就换这一段的字，然后自动跳到下一段（[SegmentState.choose] 的 advance）。
     *
     * 顶部保留一条「‹ 返回」+ 段号提示，否则用户进了展开的选字页会找不到路回去。
     */
    private fun renderExpandedSegmentRows(col: LinearLayout, state: SegmentState) {
        col.visibility = View.VISIBLE
        // 形码筛过的子集优先
        val words = if (assistActive) assistWords else state.focusedWords()
        // 第 0 行留给**操作行**（‹ 返回 / 形码 / 收起），候选从第 1 行排起。
        //
        // 为什么要这样：展开态把键盘整块藏起来了，如果这里只画候选，用户就
        // 没有任何回退入口——既回不到 chip 视图（看整句分词），也没法在不开键盘的
        // 情况下退出本次逐字选择。`collapseBar` 只能退回单行，退不回 chip。
        //
        // ## 行数为什么不能按屏幕高度截断（这里崩过一次）
        //
        // 原来是 `CandidateGrid.rows(words.size + PER_ROW, expandedRows())`——
        // 这个函数会把总行数**截到屏幕放得下的行数**，而下面的填充循环
        // `words.forEachIndexed` 仍然遍历**全部**候选，行容器数对不上就
        // `col.getChildAt(越界) as LinearLayout` → null → NPE，进程直接崩。
        //
        // 崩的后果特别难查：输入法进程被杀，组合区里那串字母被宿主刷出上屏，
        // 用户看到的是「选到第 4 个字，前面选的全没了，直接把字母串上屏」。
        // 单字候选多的（`vi` 有 99 个）必然触发，所以看起来像"特定字有问题"。
        //
        // 正解：**内容行数按候选总数算，一行不少；只把可视高度封顶，超出靠滚动。**
        // ScrollView 本来就是干这个的，`col` 高度 WRAP_CONTENT 就能滚到底。
        val gridRows = if (words.isEmpty()) 0 else
            (words.size + CandidateGrid.PER_ROW - 1) / CandidateGrid.PER_ROW
        val totalRows = gridRows + 1
        // 可视区域封顶：操作行 + 最多 expandedRows() 行候选
        val visibleRows = totalRows.coerceAtMost(expandedRows().coerceAtLeast(2))
        while (col.childCount > totalRows) col.removeViewAt(col.childCount - 1)
        while (col.childCount < totalRows) {
            col.addView(
                LinearLayout(this).apply {
                    orientation = LinearLayout.HORIZONTAL
                    gravity = Gravity.CENTER_VERTICAL
                    setPadding(dp(8f), 0, dp(8f), 0)
                },
                LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.MATCH_PARENT,
                    dp(CANDIDATE_ROW_DP),
                ),
            )
        }
        expandedScroll?.let { sv ->
            val rowH = dp(CANDIDATE_ROW_DP)
            val want = rowH * visibleRows
            (sv.layoutParams as? LinearLayout.LayoutParams)?.let { lp ->
                if (lp.height != want || lp.weight != 0f) {
                    lp.height = want
                    lp.weight = 0f
                    sv.layoutParams = lp
                }
            }
            (col.layoutParams as? FrameLayout.LayoutParams)?.let { lp ->
                if (lp.height != FrameLayout.LayoutParams.WRAP_CONTENT) {
                    lp.height = FrameLayout.LayoutParams.WRAP_CONTENT
                    col.layoutParams = lp
                }
            } ?: run {
                col.layoutParams = FrameLayout.LayoutParams(
                    FrameLayout.LayoutParams.MATCH_PARENT,
                    FrameLayout.LayoutParams.WRAP_CONTENT,
                )
            }
            sv.requestLayout()
            sv.scrollTo(0, 0)
        }

        // ── 第 0 行：操作行 ─────────────────────────────────────────
        val head = col.getChildAt(0) as LinearLayout
        head.removeAllViews()
        head.addView(
            TextView(this).apply {
                text = "‹ 返回"
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 16f)
                setTextColor(Palette.of(this@FeiheIme).sub)
                // 高度跟候选行对齐，别让操作行比候选格矮一截（点不准）
                layoutParams = LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.WRAP_CONTENT,
                    dp(CANDIDATE_ROW_DP),
                )
                setPadding(dp(10f), 0, dp(14f), 0)
                setOnClickListener {
                    // 回 chip 视图：没有「当前段」了，形码态一起退掉
                    resetAssist()
                    candidateExpanded = false
                    segmentState = state.back()
                    renderCandidates()
                }
            },
        )
        if (assistAvailable()) {
            head.addView(TextView(this).also { bindAssistView(it) })
        }
        head.addView(TextView(this).also { bindExpandToggle(it) })

        // ── 第 1 行起：候选网格 ────────────────────────────────────
        words.forEachIndexed { idx, word ->
            val row = col.getChildAt(idx / CandidateGrid.PER_ROW + 1) as LinearLayout
            val slot = idx % CandidateGrid.PER_ROW
            val v = row.getChildAt(slot) as? TextView ?: TextView(this).also {
                row.addView(it, LinearLayout.LayoutParams(0, dp(CANDIDATE_ROW_DP), 1f))
            }
            v.text = word
            v.setTextSize(TypedValue.COMPLEX_UNIT_SP, 20f)
            v.gravity = Gravity.CENTER
            v.background = null
            v.setPadding(dp(4f), 0, dp(4f), 0)
            val p = Palette.of(this)
            // 高亮按**词**比，不按下标：形码筛过之后下标和 segments 对不上
            v.setTextColor(if (word == state.wordAt(state.focus)) p.accent else p.text)
            v.setOnLongClickListener {
                val segCode = pendingSegments.getOrNull(state.focus)?.code
                if (segCode.isNullOrEmpty()) {
                    false
                } else {
                    showCandidateMenu(word, segCode, reopen = state.focus)
                    true
                }
            }
            v.setOnClickListener {
                // 按**下标**选，不按词查：候选动辄几十上百个，同字重复出现是常态
                // （`vi` 99 个候选里「只」可能不止一条），`indexOf(word)` 会定位到
                // 第一个同名字，用户点第二个「只」结果跳到第一个。
                val next = if (assistActive) {
                    state.chooseWord(word, keepOpen = true)
                } else {
                    state.choose(idx)
                }
                afterSegmentChosen(next)
            }
        }
        // 每行末尾多余的格子删掉（候选最后一行可能不满 6 个）
        for (r in 1 until totalRows) {
            val row = col.getChildAt(r) as LinearLayout
            val keep = (words.size - (r - 1) * CandidateGrid.PER_ROW).coerceIn(0, CandidateGrid.PER_ROW)
            while (row.childCount > keep) row.removeViewAt(row.childCount - 1)
        }
    }


    private fun renderExpandedRows(col: LinearLayout, window: List<String>, resetScrollHere: Boolean = true) {
        // **内容容器自己必须是 VISIBLE。**
        //
        // expandedColumn 在 onCreateInputView 里初始化为 GONE（那时还没展开），
        // 之后所有显隐操作都改的是外层 expandedScroll —— 于是展开时
        // ScrollView 可见、高度 550、里面 5 行也建好了，但内容容器是 GONE，
        // 用户看到的就是一片空白。日志特征：svH=550 而 colH=0 row0H=0。
        col.visibility = View.VISIBLE
        // **行数按实际要渲染的候选算，不做「最多几屏」截断。**
        //
        // 以前用 CandidateGrid.rows(window.size, expandedRows())，而它内部是
        // expandedCount() —— 会把数量截到 maxRows × PER_ROW（一屏的量）。
        // 展开态改成懒加载后 window 有 60 个，截成 8 行 × 5 列 = 40 格，
        // 第 41 个候选就找不到行容器 → getChildAt 返回 null → 强转 NPE 崩掉。
        // （线上表现：点展开直接闪退。）
        //
        // 现在按 window.size 算真实行数，配合外面那层 ScrollView 滚动查看，
        // 不再受屏幕高度限制。
        val rows = if (window.isEmpty()) 0 else (window.size + CandidateGrid.PER_ROW - 1) / CandidateGrid.PER_ROW
        // 行数变了就补/删行容器。每行固定高度，容器高度 = 行数 × 行高。
        while (col.childCount > rows) col.removeViewAt(col.childCount - 1)
        while (col.childCount < rows) {
            col.addView(
                LinearLayout(this).apply {
                    orientation = LinearLayout.HORIZONTAL
                    gravity = Gravity.CENTER_VERTICAL
                    setPadding(dp(8f), 0, dp(8f), 0)
                },
                LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.MATCH_PARENT,
                    dp(CANDIDATE_ROW_DP),
                ),
            )
        }
        // 展开态：整个槽位撑到「屏幕里能放下多少」。
        //
        // ## 为什么现在要精确算，而不是像以前那样让 root 自己长高
        //
        // 套上 ScrollView 之后，槽位高度就是 ScrollView 的可视高度。给太多
        // （超过 root 能拿到的）最后一行会被裁半截；给太少则下方留一片空白。
        //
        // `expandedRows()` 已经在按真实可用高度算了，这里把槽位设成
        // **那些行恰好需要的高度**；如果算出来的高度仍超出可用空间（比如
        // root 还没 measure 完），ScrollView 会接管滚动，用户不会看到半截行。
        expandedScroll?.let { sv ->
            val rowH = dp(CANDIDATE_ROW_DP)
            // **可视高度取「屏幕给多少」，不是「内容有多高」。**
            //
            // 这里原来写的是 `want = rowH * rows`，即 ScrollView 的可视高度
            // 直接等于内容高度。后果：内容永远刚好塞满可视区 → 永远滚不动，
            // 而 ScrollView 又会按这个高度向上要空间 → 候选一多就把键盘撑到
            // 占满整屏。用户看到的「15 行 4 列铺满 70% 屏幕、不是滚动」就是它。
            //
            // ScrollView 的正确用法是反过来的：
            //   可视高度 = min(内容高度, 可用高度)
            //   - 内容比可用高度少 → 收缩到内容高度，下方不留一大片空白
            //   - 内容比可用高度多 → 钉在可用高度，内部滚动
            //
            // 可用高度来自 expandedUsableHeight()（读屏幕坐标算的，没有负反馈），
            // 已经在 expandedRows() 里用了同一套，这里只是把上限也取出来。
            val contentH = rowH * rows
            val maxVisible = expandedUsableHeight() + rowH  // 补回 expandedRows 扣掉的那一行余量
            val want = if (maxVisible > 0) minOf(contentH, maxVisible) else contentH
            (sv.layoutParams as? LinearLayout.LayoutParams)?.let { lp ->
                if (lp.height != want || lp.weight != 0f) {
                    lp.height = want
                    lp.weight = 0f
                    sv.layoutParams = lp
                }
            }
            // **内容高度必须让 ScrollView 自己算（WRAP_CONTENT），不能钉死。**
            //
            // 之前这里把 expandedColumn 的高度也设成 `want`，结果 svH/colH 双双是 0，
            // 展开后整片空白。原因是：ScrollView 的子 View 用 WRAP_CONTENT 时，
            // ScrollView 才去测量内容真实高度并按需滚动；给它一个和可视区一样大的
            // 固定高度，它会认为"内容就这么高"，而内部行的 weight/measure 又依赖
            // 父级给的高度约束，形成 0 高度塌陷。
            //
            // 正确做法：容器高度 = 可视区（want），内容高度 = 自适应（WRAP_CONTENT）。
            // 内容比可视区高就滚动，比它矮就自然留白——这正是 ScrollView 该有的行为。
            // **内容必须用 FrameLayout.LayoutParams（ScrollView 的父类）。**
            //
            // 之前给 expandedColumn 挂的是 `ViewGroup.LayoutParams`——那是 addView
            // 时手写的类型。ScrollView 内部按 FrameLayout 测量子 View，
            // 拿到 ViewGroup.LayoutParams 时量不出内容高度，行高就塌成 0
            // （日志：svH=550 但 colH=0 row0H=0，容器对了、内容全空）。
            //
            // 用 MATCH_PARENT 宽 + WRAP_CONTENT 高：宽跟着 ScrollView，
            // 高由内部行自己撑开，超出可视区就滚动。
            (col.layoutParams as? FrameLayout.LayoutParams)?.let { lp ->
                if (lp.height != FrameLayout.LayoutParams.WRAP_CONTENT) {
                    lp.height = FrameLayout.LayoutParams.WRAP_CONTENT
                    col.layoutParams = lp
                }
            } ?: run {
                col.layoutParams = FrameLayout.LayoutParams(
                    FrameLayout.LayoutParams.MATCH_PARENT,
                    FrameLayout.LayoutParams.WRAP_CONTENT,
                )
            }
            // 高度改完必须重排，否则本帧读到的还是旧值（=0）
            sv.requestLayout()
            // **只在首次展开时归零滚动位置，追加批次时保持不动。**
            //
            // 懒加载滚动到底部会追加下一批（expandedLoaded 变大后重新渲染）。
            // 如果每次都 scrollTo(0,0)，用户滑到底 → 追加 → 被打回顶部 →
            // 再滑到底 → 又被打回顶部，永远看不完后面的候选。
            if (resetScrollHere) sv.scrollTo(0, 0)
        }
        window.forEachIndexed { idx, word ->
            val row = col.getChildAt(idx / CandidateGrid.PER_ROW) as LinearLayout
            val slot = idx % CandidateGrid.PER_ROW
            val v = row.getChildAt(slot) as? TextView ?: TextView(this).also {
                row.addView(it, LinearLayout.LayoutParams(0, dp(CANDIDATE_ROW_DP), 1f))
            }
            // 复用格子时要把上一轮当成占位格用过的点击态恢复回来
            v.isClickable = true
            v.isLongClickable = true
            bindCandidateView(v, word, idx == 0, code)
        }
        // 每行收尾：真候选不足 6 个的，用不可点的透明格补满。
        //
        // 为什么必须补：LinearLayout 的 weight 是在**该行实际有几个子 View**
        // 之间均分的。第一行 6 个各得 1/6，最后一行只有 4 个就各得 1/4 ——
        // 最后一行明显比上面宽，整片候选区左右不对齐，看着像排版坏了。
        // 补满 6 个之后每列宽度一致，行列才对得齐。
        for (r in 0 until rows) {
            val row = col.getChildAt(r) as LinearLayout
            val keep = minOf(CandidateGrid.PER_ROW, window.size - r * CandidateGrid.PER_ROW)
            while (row.childCount > CandidateGrid.PER_ROW) row.removeViewAt(row.childCount - 1)
            while (row.childCount < CandidateGrid.PER_ROW) row.addView(newPlaceholderCell())
            // keep 之后的位置都是占位格：清干净，别让上一页的字残留在那儿
            for (c in keep until row.childCount) {
                (row.getChildAt(c) as? TextView)?.let { ph ->
                    ph.text = ""
                    ph.background = null
                    ph.isClickable = false
                    ph.isLongClickable = false
                    ph.setOnClickListener(null)
                    ph.setOnLongClickListener(null)
                }
            }
        }
    }

    /** 展开态用来把每行补满 6 格的透明占位：只为等宽，不可点、无内容。 */
    private fun newPlaceholderCell(): TextView =
        TextView(this).apply {
            layoutParams = LinearLayout.LayoutParams(0, dp(CANDIDATE_ROW_DP), 1f)
            isClickable = false
            isLongClickable = false
            text = ""
        }

    /**
     * 改候选条容器的高度。
     *
     * 输入视图内部改高度**不会**触发宿主 resize（我们一直在同一个 InputView 里动子
     * View），所以这里可以放心按行数撑高。这条是「展开候选不把微信会话列表顶得乱跳」
     * 的关键——真正的窗口 resize 才会重排宿主界面。
     */
    private fun resizeCandidateHost(rows: Int) {
        val scroll = candidateScroll ?: return
        val lp = scroll.layoutParams ?: return
        // 展开态：候选区吃掉键盘腾出的整块高度。这里必须用**显式像素**而不是
        // MATCH_PARENT——纵向 LinearLayout 里 MATCH_PARENT 会按可用空间收缩，
        // 而 HorizontalScrollView 又只按自身高度裁剪子 View，结果就是「多行被裁掉，
        // 只剩第一行」。行数 × 行高算出来的高度才是真能显示出来的高度。
        val want = if (rows <= 0) {
            val n = expandedRows()
            dp(CANDIDATE_ROW_DP * n)
        } else {
            dp(CANDIDATE_ROW_DP * rows)
        }
        if (lp.height != want) {
            lp.height = want
            scroll.layoutParams = lp
        }
    }

    /**
     * 展开态把键盘区整块收起，把屏幕让给候选。
     *
     * 用户要的就是这个：候选多的时候，底下的字母键一点用都没有（选词又不靠它），
     * 却占掉小半个屏幕。展开时把它藏掉，候选区就能铺七八行。
     *
     * 收起态必须原样还原：keyboardHost 是 GONE 的话整个输入法只剩一条候选栏，
     * 用户会以为输入法坏了。所以这是个对称的开关，不是单向的 show/hide。
     */
    private fun applyExpandedChrome(expanded: Boolean) {
        val kh = keyboardHost ?: return
        // 剪贴板面板打开时，键盘是故意藏起来的。这里如果无条件 VISIBLE，
        // 候选一刷新（打字、切页、读剪贴板）就会把键盘顶回剪贴板面板上，
        // 表现是「点了剪贴板，面板闪一下就没了」。剪贴板态优先。
        if (clipOpen) {
            collapseBar?.visibility = View.GONE
            kh.visibility = View.GONE
            numberRowHost?.visibility = View.GONE
            return
        }
        val nr = numberRowHost
        collapseBar?.visibility = if (expanded) View.VISIBLE else View.GONE
        if (expanded) {
            kh.visibility = View.GONE
            nr?.visibility = View.GONE
        } else {
            kh.visibility = View.VISIBLE
            // 数字选词行原本就有自己的显隐逻辑（没候选时 GONE），
            // 这里不能无脑 VISIBLE，否则没候选时会冒出一条空行。
            nr?.let { it.visibility = if (it.childCount > 0) View.VISIBLE else View.GONE }
        }
    }

    /** 展开 / 收起按钮。图标用 ▾ / ▴，点一下切换。 */
    private fun bindExpandToggle(v: TextView) {
        // **文案要能表达「还有更多」。**
        //
        // 收起态一屏只放三五个候选（这样渲染才不卡），但一个字可能有几百个候选
        // （实测 l=565、li=281）。只画一个 ▾ 箭头，用户根本不知道点开有多少东西，
        // 多半以为候选就这么几个、输入法词库不行。
        //
        // 所以有剩余候选时显示「▾ 565」，把总数直接摆出来：既说明还有多少，
        // 也成了「想看全量就往这儿点」的入口。
        val total = candidates.size
        v.text = when {
            candidateExpanded -> "▴ 收起"
            total > 0 -> "▾ $total"
            else -> "▾"
        }
        v.setTextSize(TypedValue.COMPLEX_UNIT_SP, if (candidateExpanded) 12f else 13f)
        // **必须画出可见的边框。**
        //
        // 用户反馈「展开按钮没有边框，不确定点击范围，容易点不到」。原先
        // `background = null` + 撑 minWidth，实际是**一块看不见的透明区**：
        // 手指不知道该往哪儿落，点偏一点就落到旁边的候选词上，表现为
        // 「点展开没反应」（其实是选了隔壁那个字）。
        //
        // 描边 + 圆角和分段态的 chip 用同一套（chipStroke / 4dp），
        // 整个候选条的视觉语言保持统一。
        // 不调 setColor：默认透明填充。见 bindAssistView 里的说明——
        // 显式设全透明色会让部分系统版本跳过整张 shape 的绘制。
        v.background = GradientDrawable().apply {
            setStroke(dp(CHIP_STROKE_DP), Palette.of(this@FeiheIme).chipStroke)
            cornerRadius = dp(4f).toFloat()
        }
        // **点击区要够大**：13sp 的字加 padding 只有约 30dp 宽，手指点上去
        // 经常落到相邻的候选词上。横向撑到 44dp，是安卓无障碍建议的最小可点尺寸。
        //
        // 注意 minWidth 在 LinearLayout 的 weight 布局里不一定生效（父容器按
        // weight 分配宽度时会覆盖它），所以真正的保障是下面这行
        // minimumWidth —— minWidth 管的是 TextView 自己的 measure 建议值，
        // minimumWidth 才是「无论父容器怎么量，至少这么宽」的硬约束。
        v.minWidth = dp(48f)
        v.minimumWidth = dp(48f)
        // **高度也必须显式给。**
        //
        // 父容器是 `bar.addView(TextView(this))` / `head.addView(...)`，
        // 都没带 LayoutParams，于是 TextView 按 WRAP_CONTENT 量——
        // 13sp 的字加零垂直 padding 只有约 17dp 高。用户反馈「高度很窄，
        // 不好点」就是它：宽度撑到了 48dp，高度还是个字高。
        //
        // 钉成一行候选的标准行高（CANDIDATE_ROW_DP = 40dp），
        // 和候选格同高之后整条候选栏的基线才对得齐。
        v.layoutParams = LinearLayout.LayoutParams(
            LinearLayout.LayoutParams.WRAP_CONTENT,
            dp(CANDIDATE_ROW_DP),
        )
        v.gravity = Gravity.CENTER
        v.setPadding(dp(10f), 0, dp(10f), 0)
        v.setTextColor(Palette.of(this).sub)
        v.contentDescription =
            if (candidateExpanded) "收起候选" else "展开候选，共 $total 个"
        v.setOnClickListener {
            candidateExpanded = !candidateExpanded
            renderCandidates()
        }
    }

    /**
     * 一页几个候选。
     *
     * 收起态 = 设置里的「候选个数」（5/7/9）；展开态 = 行 × 列（见 [CandidateGrid]）。
     * 两个模式都从这里取，保证展开/收起切换时窗口计算一致。
     */
    /**
     * 展开态能放几行。
     *
     * ## 为什么不能拿「当前 root 高度」算（这是「展开只有三行」的根因）
     *
     * root 是 `LinearLayout(VERTICAL)` 且没钉高度——系统按内容 WRAP_CONTENT 测量。
     * 展开后键盘是 GONE 的，root 高度 ≈ `expandedScroll.want + 状态条 + 收起条`，
     * 而 `want = rowH × rows`。于是：
     *
     * ```
     * rows = f(root)      root = rowH*rows + chrome
     * => usable = rowH*(rows-1) - chrome  =>  rows' = rows - 1
     * ```
     *
     * **每渲染一次就少一行**，一路跌到 1 才停。模拟器上因为某次 measure 时机不同
     * 停在 5 行，真机上停在 3 行——同一份代码两种表现，全靠运气。
     *
     * 解法：不读 root，读**屏幕坐标**。键盘顶部的位置（`windowVisibleDisplayFrame`
     * 的 bottom）是系统给的，和我们内部怎么排版完全无关，因此没有反馈回路。
     */
    private fun expandedRows(): Int {
        val rowH = dp(CANDIDATE_ROW_DP)
        if (rowH <= 0) return CandidateGrid.MAX_ROWS
        val usable = expandedUsableHeight()
        if (usable <= 0) return CandidateGrid.MAX_ROWS
        return (usable / rowH).coerceIn(1, 8)
    }

    /**
     * 展开态真正能铺候选的像素高度。
     *
     * = 键盘顶部以上的可用区 − 状态条 − 收起条 − 一行余量（免得最后一行裁半截）。
     */
    private fun expandedUsableHeight(): Int {
        val rowH = dp(CANDIDATE_ROW_DP)
        val screenH = resources.displayMetrics.heightPixels
        // 键盘顶部的 y 坐标 = 系统「可见显示区」的下边界。
        // 拿不到（未 attach / 全屏沉浸）就退回屏幕高度的 55% 估算——
        // 键盘一般占屏幕 45%~55%，这是老代码就在用的兜底值。
        val keyboardTop = runCatching {
            val r = android.graphics.Rect()
            candidateScroll?.rootView?.getWindowVisibleDisplayFrame(r)
            if (r.bottom > 0 && r.bottom < screenH) r.bottom else 0
        }.getOrDefault(0).takeIf { it > 0 } ?: (screenH * 0.55f).toInt()
        var chrome = 0
        statusBar?.let { if (it.height > 0) chrome += it.height else chrome += dp(34f) }
        chrome += dp(30f) // 收起条自己
        return keyboardTop - chrome - rowH
    }

    /**
     * 输入法视图的真实高度。
     *
     * root 在 onCreateInputView 里返回、被系统 measure 之后才有 height。
     * 还没 measure 时返回 0，调用方退回估算。
     */
    private fun rootViewHeight(): Int {
        val v = candidateScroll?.rootView ?: return 0
        return if (v.height > 0) v.height else 0
    }

    /**
     * 收起态一页几个候选。
     *
     * **这是「打字卡顿」的修复点。** 原先传 `Int.MAX_VALUE`，等于告诉分页逻辑
     * 「一页装得下全部候选」，于是 `window()` 返回 281 个，`renderCandidates`
     * 为每一个建 TextView —— 实测单键 170ms，其中 168ms 在渲染。
     *
     * 现在按屏宽算出一屏实际放得下的个数（3~7，见 [CandidateGrid.collapseSlotCount]），
     * 候选总数仍然不设上限：全部候选都在 `candidates` 里，靠 ‹ › 翻页逐个拿到。
     * 用户要的「无限候选」和「不卡」在这里并不冲突——卡的是渲染，不是候选数量。
     */
    private fun collapsePageSize(): Int {
        val dm = resources.displayMetrics
        val widthDp = (dm.widthPixels / dm.density).toInt()
        val fontScale = if (dm.density > 0f) dm.scaledDensity / dm.density else 1f
        return CandidateGrid.collapseSlotCount(
            screenWidthDp = widthDp,
            textSizeDp = 20f * fontScale,
        )
    }

    /**
     * 一页几个候选。收起态单行 + 展开态多行，两种模式的页窗口计算。
     *
     * 展开态本来就是一屏铺满（行 × 列），继续按实际能放的行数算。
     */
    private fun pageSize(): Int =
        CandidateGrid.pageSize(
            candidates.size,
            candidateExpanded,
            collapsePageSize(),
            expandedRows(),
        )

    /**
     * 当前要渲染的候选。
     *
     * - **收起态**：按屏宽分页，一次只给一屏（3~5 个）。这是不卡的关键——
     *   原先给全部，一个字 565 个候选就要建 565 个 View，卡 170ms。
     * - **展开态**：从第一个开始给「已加载」那么多（懒加载，初始 60 个），
     *   整片放进 ScrollView 连续滚，滑到底再追加。展开的语义是「摊开看全部」，
     *   所以不分页——565 个分 14 页翻比滚动更累。
     */
    private fun visibleCandidates(): List<String> {
        if (candidateExpanded) {
            val n = CandidateGrid.expandedLoaded(candidates.size, expandedLoaded)
            return if (n <= 0) emptyList() else candidates.subList(0, n)
        }
        return CandidateGrid.window(
            candidates,
            candPage,
            candidates.size,
            expanded = false,
            collapseSize = collapsePageSize(),
            maxRows = expandedRows(),
        )
    }

    /** 翻页箭头。`delta` = -1 上一页 / +1 下一页。 */
    private fun bindPageView(v: TextView, label: String, delta: Int) {
        v.background = null
        v.text = label
        v.setTextSize(TypedValue.COMPLEX_UNIT_SP, 18f)
        v.setPadding(dp(10f), 0, dp(10f), 0)
        v.setTextColor(Palette.of(this).sub)
        v.setOnClickListener {
            candPage = (candPage + delta).coerceAtLeast(0)
            renderCandidates()
        }
    }

    /** 候选条上的说明文字（形码筛空、无更多页等），不可点。 */
    private fun bindHintView(v: TextView, text: String) {
        v.background = null
        v.text = text
        v.setTextSize(TypedValue.COMPLEX_UNIT_SP, 13f)
        v.setPadding(dp(10f), 0, dp(10f), 0)
        v.setTextColor(Palette.of(this).hint)
        v.setOnClickListener(null)
    }

    /**
     * 顶部那一行显示谁：有输入 / 候选 → 候选条；什么都没在打 → 状态栏。
     *
     * 两者**共用一个固定高度的槽**，不是各占一行。窗口高度一变，微信 / QQ 就会
     * 重排会话列表，表现为每敲一下闪一下。
     */
    private fun updateTopSlot() {
        val typing = quickMode || pending.isNotEmpty() || candidates.isNotEmpty() ||
            suggestions.isNotEmpty() || segmentState != null || otpCode != null
        // 语音占着这个槽位时（录音/识别/加载模型），候选和状态条都不该露面：
        // 三者是同一个位置，同时可见会叠在一起。
        val voiceBusy = voiceState == VoiceRecorder.State.RECORDING ||
            voiceState == VoiceRecorder.State.RECOGNIZING ||
            voiceState == VoiceRecorder.State.LOADING
        if (voiceBusy) {
            // **只让出候选条，不能让状态条消失。**
            //
            // 状态条上有麦克风按钮和「正在录音，再点一下结束」这句提示：
            // 隐藏它等于把唯一的操作入口和反馈一起藏了，用户只能等录音自己结束。
            // 波形和候选条是同一个槽位，两者才需要互斥。
            statusBar?.visibility = View.VISIBLE
            candidateScroll?.visibility = View.GONE
            expandedColumn?.visibility = View.GONE
            return
        }
        statusBar?.visibility = if (typing) View.GONE else View.VISIBLE
        // 展开态下这一槽由 expandedColumn 接管，candidateScroll 必须保持 GONE，
        // 否则两个容器同时 VISIBLE，一个固定一行高、一个吃掉剩余高度，视觉上叠在一起。
        candidateScroll?.visibility =
            if (typing && !candidateExpanded) View.VISIBLE else View.GONE
    }

    /** 普通候选项。每个视觉属性都重设：这一格可能是从分段 chip 复用来的，带背景。 */
    private fun bindCandidateView(
        v: TextView,
        word: String,
        first: Boolean,
        pinCode: String? = null,
    ) {
        v.background = null
        v.text = word
        v.setTextSize(TypedValue.COMPLEX_UNIT_SP, 20f)
        // **候选词绝不允许折行。**
        //
        // 展开态每个格子宽度 = 屏宽 / 6（约 60dp），而两字词「智慧」在 20sp 下
        // 约需 44dp，加上左右各 14dp 的 padding 就是 72dp —— 超了，TextView 会
        // 把第二个字折到下一行。用户看到的就是「两字词竖着排成两行」：
        // 上面「智」下面「慧」，六个格子各占一格，整片候选区像乱码。
        //
        // 这是去掉候选上限、候选变多之后的连带问题：以前一页只有 5~9 个，
        // 单行横滑时格子宽度按内容自适应，两字词不会被挤；改成 6 列固定等分后
        // 才暴露。修法是**单行 + 省略号 + 收紧 padding**，三件事缺一不可：
        // 只加单行不收紧 padding，字会被省略成「智…」；只收紧不给单行，
        // 宽度一不够又折行。
        v.maxLines = 1
        v.isSingleLine = true
        v.ellipsize = android.text.TextUtils.TruncateAt.END
        // 展开态 6 列时左右各 14dp 太挤（168dp 全被 padding 吃掉），收到 6dp。
        // 单行横滑时格子是按内容 WRAP_CONTENT 的，6dp 也足够透气。
        //
        // **必须用 CandidateGrid.CELL_PADDING_DP / 2**：候选格子宽度的判据
        // （`willTruncate`）读的是同一个常量。两边各写一个数字的话，测试会
        // 绿着而线上折行 —— 这个 bug 就是这么逃过去的。
        val cellPad = CandidateGrid.CELL_PADDING_DP / 2
        v.setPadding(dp(cellPad.toFloat()), 0, dp(cellPad.toFloat()), 0)
        // 第一个候选高亮：它对应空格，得让人看出来
        val p = Palette.of(this)
        v.setTextColor(if (first) p.accent else p.text)
        v.setOnClickListener { commitWord(word) }
        // 长按置顶/降权。手机上这是**唯一**纠正候选顺序的手势（没有右键、
        // 没有 Ctrl+点击）。没有码的候选（快捷输入结果）不给菜单——它不属于词库。
        v.setOnLongClickListener {
            if (pinCode == null) {
                false
            } else {
                showCandidateMenu(word, pinCode)
                true
            }
        }
    }

    /**
     * 长按候选弹出的菜单：置顶 / 降权。
     *
     * 两项放一个弹窗里，而不是做成两种手势：手机候选条只有单击可用，再切一个
     * 「长按 + 滑动」之类的组合没人记得住。选完立刻重查，用户能马上看到这一项挪到哪。
     *
     * [reopen] ≥ 0 时（分段视图里长按）重查后把这一段重新展开——否则用户刚改完
     * 一段，界面就跳回 chip 视图，还得再点一次。
     */
    private fun showCandidateMenu(word: String, code: String, reopen: Int = -1) {
        AlertDialog.Builder(this)
            .setTitle(word)
            .setItems(arrayOf("置顶", "降权")) { _, which ->
                val on = if (which == 0) {
                    NativeEngine.togglePin(code, word)
                } else {
                    NativeEngine.toggleDemote(code, word)
                }
                Toast.makeText(
                    this,
                    if (which == 0) {
                        if (on) "已置顶「$word」" else "已取消置顶"
                    } else {
                        if (on) "已降权「$word」" else "已取消降权"
                    },
                    Toast.LENGTH_SHORT,
                ).show()
                refreshCandidates()
                if (reopen >= 0) {
                    segmentState = segmentState?.open(reopen)
                    renderCandidates()
                }
            }
            .show()
    }

    /**
     * 验证码提示。**文案必须带「验证码」三个字**：候选条平时是一排汉字，单独一个
     * "123456" 混在里面，用户会以为那是词库里的词，不敢点。
     */
    private fun bindOtpView(v: TextView, code: String) {
        v.background = null
        v.text = "验证码 $code"
        v.setTextSize(TypedValue.COMPLEX_UNIT_SP, 18f)
        v.setPadding(dp(12f), 0, dp(12f), 0)
        v.setTextColor(Palette.of(this).accent)
        v.setOnClickListener { commitOtp(code) }
    }

    private fun commitOtp(code: String) {
        otpCode = null
        clearComposing()
        commitText(code)
        refreshCandidates()
    }

    /**
     * 候选条靠左的 😊 入口（形/验证码之后）：点一下进表情页。
     *
     * 视觉规格和「形」按钮一致（chip 底色 + 描边 + 4dp 圆角），因为它们是**同一排
     * 里的同类控件**——凡是有框的都是能点的，没框的是候选文字。宽度钉死 44dp，
     * 避免 HorizontalScrollView 下被量成 0（这是「形」按钮踩过的坑）。
     */
    private fun bindEmojiChip(v: TextView) {
        val p = Palette.of(this)
        v.text = "😊"
        v.setTextSize(TypedValue.COMPLEX_UNIT_SP, 16f)
        v.gravity = Gravity.CENTER
        v.setPadding(0, 0, 0, 0)
        v.background = GradientDrawable().apply {
            setColor(p.chipBg)
            setStroke(dp(CHIP_STROKE_DP), p.chipStroke)
            cornerRadius = dp(4f).toFloat()
        }
        v.layoutParams = LinearLayout.LayoutParams(dp(44f), dp(32f)).apply {
            marginStart = dp(4f)
            marginEnd = dp(4f)
        }
        v.contentDescription = "表情"
        v.isClickable = true
        v.setOnClickListener { showPage(Page.EMOJI) }
        v.setOnLongClickListener(null)
    }

    /**
     * 「形」按钮。**文案里必须带上已打的形码**（`形 tl`）：形码是看不见的状态，
     * 只亮一个按钮的话，用户不知道刚才那两个键到底吃进去没有。
     */
    private fun bindAssistView(v: TextView) {
        val p = Palette.of(this)
        v.text = when {
            !assistActive -> "形"
            assistKeys.isEmpty() -> "形 _"
            else -> "形 $assistKeys"
        }
        v.setTextSize(TypedValue.COMPLEX_UNIT_SP, 15f)
        // **给「形」一个看得见的框。**
        //
        // 它是进/出形码态的开关，但原先 `background = null`——在候选条一排字里
        // 和候选词长得一模一样，用户根本分不清「哪个是能点的按钮、哪个是候选字」。
        // 点偏一格就选了隔壁那个字。
        //
        // 描边和展开按钮、「收起候选」胶囊用同一个 chipStroke，三处视觉一致：
        // 凡是有框的都是控件，没框的都是候选文字。
        // **不要调 setColor**：GradientDrawable 默认就是透明填充，
        // 显式 setColor(0x00000000)（全透明）在部分 Android 版本上会让整张
        // shape 的绘制被跳过——描边跟着一起消失，表现就是「设了边框但看不见」。
        // 分段态那批 chip 一直可见，正是因为它们从来没设过 color。
        // **填充 + 描边双保险。**
        //
        // 只画 1.5dp 描边在 440dpi 屏上只有 4px，混在一排文字里几乎看不出边界
        // （实测截图放大到 4 倍才能辨认）。所以再给一层很浅的填充色，
        // 让「这是一个控件」这件事在正常观看距离下就能感知到。
        //
        // 填充用 chipStroke 的极淡版本：肉眼能看到色块，又不会抢候选词的注意力。
        v.background = GradientDrawable().apply {
            setColor(if (assistActive) p.accentBg else p.chipBg)
            setStroke(dp(CHIP_STROKE_DP), if (assistActive) p.accent else p.chipStroke)
            cornerRadius = dp(4f).toFloat()
        }
        // **必须显式给 LayoutParams 一个固定宽度。**
        //
        // 日志实测 bind 时 `w=0`，点击坐标落在按钮上看不到任何反应——
        // 说明它不是「还没 measure」，而是**真的被量成了 0 宽**。
        // minWidth/minimumWidth 对 WRAP_CONTENT 的 TextView 只是「建议值」，
        // 父容器 HorizontalScrollView 里的 LinearLayout 在分配时不一定采纳。
        //
        // 所以直接把宽度钉死：48dp 是安卓无障碍建议的最小可点尺寸。
        v.gravity = Gravity.CENTER
        v.setPadding(dp(10f), 0, dp(10f), 0)
        v.layoutParams = LinearLayout.LayoutParams(dp(48f), dp(CANDIDATE_ROW_DP))
        v.setTextColor(if (assistActive) p.accent else p.sub)
        v.contentDescription = if (assistActive) "退出形码" else "进入形码"
        v.setOnClickListener { toggleAssist() }
    }

    /** 候选和联想之间的细竖线，不可点。 */
    private fun bindSeparatorView(v: TextView) {
        v.background = null
        v.text = "|"
        v.setTextSize(TypedValue.COMPLEX_UNIT_SP, 16f)
        v.setPadding(dp(2f), 0, dp(2f), 0)
        v.setTextColor(Palette.of(this).separator)
        v.setOnClickListener(null)
    }

    /** 联想项。**加 → 前缀不是为了好看**：必须一眼看出「点了直接上屏、不吃空格」。 */
    private fun bindSuggestionView(v: TextView, word: String) {
        v.background = null
        v.text = "→$word"
        v.setTextSize(TypedValue.COMPLEX_UNIT_SP, 20f)
        v.setPadding(dp(14f), 0, dp(14f), 0)
        v.setTextColor(Palette.of(this).sub)
        v.setOnClickListener { commitSuggestion(word) }
    }

    /**
     * 邮箱后缀候选：点了就把 `@qq.com` 接在那串数字后面。
     *
     * 用 18sp 而不是联想项的 20sp，是为了让 `@163.com` 这种长一点的后缀在候选条上
     * 不至于挤成两行；颜色比联想项亮一档（主色），因为它是「补全当前输入」而不是
     * 「接着往下写」，语义更接近候选。
     *
     * 字号不跟随系统字体缩放的坑这里不涉及——输入法自己控制视图，不接系统字号。
     */
    private fun bindEmailSuffixView(v: TextView, suffix: String) {
        v.background = null
        v.text = suffix
        v.setTextSize(TypedValue.COMPLEX_UNIT_SP, 18f)
        v.setPadding(dp(12f), 0, dp(12f), 0)
        v.setTextColor(Palette.of(this).accent)
        v.contentDescription = "补全邮箱后缀 $suffix"
        v.setOnClickListener { commitEmailSuffix(suffix) }
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
                    setTextColor(Palette.of(this@FeiheIme).sub)
                    setPadding(dp(10f), 0, dp(14f), 0)
                    setOnClickListener {
                        // 回到 chip 视图 = 没有「当前段」了，形码态一起退掉：
                        // 留着「亮着但没目标」的按钮，用户会以为它坏了。
                        resetAssist()
                        segmentState = state.back()
                        renderCandidates()
                    }
                },
            )
            // 形码按钮：分段态下它筛的是**当前这一段**，所以只在展开某一段时出现。
            if (assistAvailable()) {
                bar.addView(TextView(this).also { bindAssistView(it) })
            }
            // **展开按钮**：分段选字时一屏只有六七个字，逐字挑词要滑很久。
            //
            // 以前这里没有展开入口，而 `bindExpandToggle` 只挂在
            // `renderCandidates` 的单行普通候选分支里——分段态在更早的分支就
            // `return` 了，根本走不到那里。所以用户进了逐字选词之后，
            // 屏幕上**没有任何办法**变成多行，这正是他报的「组词退格进选单字
            // 时候依然没有展开成多行的途径」。
            //
            // 放在「‹ 返回」和形码之后：这两个是状态位/退路，要压在左边。
            bar.addView(TextView(this).also { bindExpandToggle(it) })
            // 形码态下候选是筛过的子集，**必须用它**（哪怕是空表）：拿未筛的列表顶
            // 上去，用户会以为形码没生效。
            val words = if (assistActive) assistWords else state.focusedWords()
            for (word in words) {
                bar.addView(
                    TextView(this).apply {
                        text = word
                        setTextSize(TypedValue.COMPLEX_UNIT_SP, 20f)
                        setPadding(dp(12f), 0, dp(12f), 0)
                        val p = Palette.of(this@FeiheIme)
                        // 高亮按**词**比，不按下标：筛选后下标和 segments 对不上
                        setTextColor(if (word == state.wordAt(state.focus)) p.accent else p.text)
                        // 长按置顶/降权：分段态的码是**这一段的**，不是整串
                        setOnLongClickListener {
                            val segCode = pendingSegments.getOrNull(state.focus)?.code
                            if (segCode.isNullOrEmpty()) {
                                false
                            } else {
                                showCandidateMenu(word, segCode, reopen = state.focus)
                                true
                            }
                        }
                        setOnClickListener {
                            if (assistActive) {
                                // 按词选（下标是筛过的），选完自动展开下一段
                                val next = state.chooseWord(word, keepOpen = true)
                                afterSegmentChosen(next)
                            } else {
                                // 未筛选时下标就在当前段里，直接换算回去。
                                // advance=true：选完自动展开下一段，用户可以连着挑下一个字。
                                val idx = state.focusedWords().indexOf(word)
                                val next = state.choose(idx)
                                afterSegmentChosen(next)
                            }
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
                    run {
                        val p = Palette.of(this@FeiheIme)
                        setTextColor(if (changed) p.accent else p.text)
                    }
                    // 分段之间画一道竖线，否则「张三李」三个 chip 会看成一个词
                    background = GradientDrawable().apply {
                        setStroke(dp(1f), Palette.of(this@FeiheIme).chipStroke)
                        cornerRadius = dp(4f).toFloat()
                    }
                    setOnClickListener {
                        segmentState = state.open(i)
                        // 换一段 = 换一个字，上一段的形码不能再筛这一段
                        assistKeys.setLength(0)
                        updateAssistWords()
                        renderCandidates()
                    }
                },
            )
        }
        bar.addView(
            TextView(this).apply {
                text = "  ↵上屏"
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 13f)
                setTextColor(Palette.of(this@FeiheIme).hint)
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
        // 快捷输入态下数字行要跟着 quickWords 走：只打一个「/」后面排着十几条命令，
        // 没有数字行就只能用手指一条条点，这是 Windows 端不必忍受的事。
        val shown = if (quickMode) quickWords.size else visibleCandidates().size
        val rows = if (showNumberRow && !segmentActive) {
            NumberSelectLayout.rows(shown)
        } else {
            emptyList()
        }
        // 内容只由「显示开关 + 候选个数」决定，没变就不重建。打一个词要按五六个键，
        // 候选个数常常不变，这里省掉的是每次按键 10 来个 View + 9 个 GradientDrawable。
        val key = if (rows.isEmpty()) 0 else shown + 1
        if (key == numberRowKey) return
        numberRowKey = key
        host.removeAllViews()
        // **不再 GONE**：高度是固定的（见 [NUM_ROW_DP]），没候选时清空内容即可。
        // 一 GONE 一 VISIBLE 就是一次窗口高度变化，宿主会跟着重排。
        if (rows.isEmpty()) {
            return
        }
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
     * 退格把「整词」切成「逐字」：判据和 Windows 端 `backspace_splits` 完全一致。
     *
     * 打 `xrji` 出「璇玑」，用户想要的却是「璇」+ 别的字 —— 按一次退格切成
     * `[璇][玑]`，编码 `xrji` 一个字都不删。这是品牌输入法和桌面端共有的手感，
     * 不是「删音节」。
     *
     * 三个判据（照抄 Windows，别自己发挥）：
     * 1. **没有候选** → 不退格拆字，交给调用方删字母（用户还没打完）
     * 2. **第一候选是单字**（`de` → 的）→ 这码本来就是「一堆同码字」，
     *    退格拆字毫无意义，别劫持
     * 3. **第一候选是多字词** → 说明这串码被一个词占满了，该给用户换字的机会
     *
     * @return true = 已进入分段态，调用方不要再动 `pending`
     */
    private fun backspaceIntoSegments(): Boolean {
        // 已经在分段态由上面的分支处理，这里只负责「从整词切进来」
        if (segmentState != null) return false
        // 判据和 Windows 端同一份（`BackspaceSplit.shouldSplit`）：
        // 第一候选是单字就不拆（`de` → 的 这种码本来就该换候选）
        if (!BackspaceSplit.shouldSplit(visibleCandidates())) return false
        // **按需向引擎要分段，不要用 `pendingSegments`。**
        //
        // `pendingSegments` 是 `lookupInput` 搭车带回来的，只在「整码查不到」时非空。
        // 用户按退格要拆的那个词恰恰是整码查得到的（「河西」就摆在那），
        // 所以 `pendingSegments` 恒为空——以前直接拿它构造 SegmentState，
        // `active` 为 false，退格落到删字母，把 `hexi` 吃成 `hex`。
        // **一律按音节向引擎要，不要复用 `pendingSegments`。**
        //
        // 那份是 `lookupInput` 搭车带回来的，用的是 **Word 粒度**（贪心最长词段），
        // `lqxbrj` 会被切成 `[lqxb][rj]` —— 中间那个「心」并进了两字词里，
        // 用户想单独换掉它没有任何入口。这正是「三个字拆不开」的直接原因。
        //
        // 退格的意图是「我要逐字改」，所以必须用 Syllable 粒度重新要一次：
        // 双拼下两个字母恰好一个字，纯粹按位置切，不查词库不做 DP。
        val raw = NativeEngine.segments(
            pending.toString(),
            SEG_CAND_LIMIT,
            bySyllable = true,
        )
        if (raw.size < 2 || raw.any { it.second.isEmpty() }) return false
        val next = SegmentState(
            segments = raw.map { it.second },
            // **必须带码**：逐字选词是用户造词的主要路径，上屏时要靠它算全拼。
            codes = raw.map { it.first },
        ).takeIf { it.active } ?: return false
        // **退格进来的第一段直接展开**：用户的意图就是「换个字」，还要他再点一下
        // chip 才能看候选是白折腾。这一条是他明确要求的「退格进入选字模式」。
        segmentState = next.open(0)
        segmentStateCode = pending.toString()
        renderCandidates()
        return true
    }

    /**
     * 词库里没有的词，记成自造词，下次同码就能直接出现在候选里。
     *
     * 为什么需要这一步：`NativeEngine.learn`（`freq::bump`）只能给**已有候选**加次数、
     * 改排序。用户打「飞鹤」这种词库里没有的组合时，「和」是选了才上屏的，
     * 「鹤」也是，但**「飞鹤」这个整体从来没有进过候选表**——`freq` 记多少次都不会
     * 让它凭空出现。这就是「组过的词下次还得重新组」的根因。
     *
     * Windows 端靠 `compose_phrase` 走分段流程解决；Android 没有分段 UI，走的是
     * 「打码 → 从候选里挑字」，所以判定条件换成：**这个词库里查不到，就按自造词记**。
     *
     * 全拼必须反查得到，任何一环失败就整体放弃——`phrase` 按全拼建索引，拿不到
     * 可靠的全拼，记下来的词换方案/切全拼之后就再也打不出来，比不记更糟。
     */
    private fun rememberPhraseIfNew(code: String, word: String) {
        // 单字不叫「组词」：单字的排序已经由 `freq` 管，记成自造词只会灌垃圾。
        if (code.isEmpty() || word.length < 2) return
        // 词库里查得到 → 不是新词，别记（会给表里灌重复数据）
        if (NativeEngine.pinyinOf(code, word).isNotEmpty()) return
        // 反查每个字的全拼。拿不到就放弃：存不进全拼，这个词以后就捞不出来。
        val syllables = StringBuilder()
        var i = 0
        while (i < word.length) {
            // 按 code point 走，别把 emoji / 生僻字的代理对劈开
            val cp = word.codePointAt(i)
            val ch = String(Character.toChars(cp))
            i += Character.charCount(cp)
            val py = NativeEngine.pinyinOf(syllableCodeFor(ch), ch)
            if (py.isEmpty()) return
            if (syllables.isNotEmpty()) syllables.append(' ')
            syllables.append(py)
        }
        NativeEngine.learnPhrase(word, syllables.toString())
    }

    /**
     * 一个字单独打出来时的码。双拼下是它的两个键，全拼下就是它的拼音。
     *
     * 用 `encodePhrase`（引擎里和 Windows 端同一套逻辑）而不是自己拼字符，
     * 方案切换（小鹤/微软）时不用改这里。
     */
    private fun syllableCodeFor(ch: String): String =
        NativeEngine.encodePhrase(ch).ifEmpty { ch }

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
        // 按「码 → 词」记一次频次：这是用户真正选了这个词，下次同码它要往前排。
        // **必须在下面 `code = ""` 之前调**——这里 code 还是本次查询用的码。
        // 以前只调 record（记词对），选词本身从没被记住，所以自造词永远排不上来。
        NativeEngine.learn(code, word)
        // 词库里没有的词要单独记成「自造词」，否则下次它连候选都进不来（见下）。
        rememberPhraseIfNew(code, word)

        pending.setLength(0)
        code = ""
        candidates = emptyList()
        pendingSegments = emptyList()
        // 整句上屏后组合结束，分段态必须一起收掉，否则候选条会留着一排旧 chip
        segmentState = null
        segmentStateCode = ""
        // 形码跟着这次组合一起结束：留着的话下一个词的候选会被上一词的形码筛空
        resetAssist()
        candPage = 0
        // **展开态必须一起收掉。**
        //
        // 用户报：「展开选字，选完最后一个字，候选区域空了但还留着一个约占一行
        // 高度的空白面板，底下还有一个『收起』的点击区，还得手动再点一次。」
        //
        // 根因：上屏把候选数据清空了（candidates/segmentState 都置空），
        // 但 `candidateExpanded` 这个**纯呈现标志**没人动。于是 renderCandidates
        // 仍然按展开态走：applyExpandedChrome(true) 继续把键盘藏起来、候选区
        // 按 expandedRows() 撑着高度——可里面已经没有候选可画，只剩操作行
        // （‹ 返回 / 收起）。用户看到的就是一块空白 + 一根多余的收起条。
        //
        // 这不是分段态独有的问题：普通拼音展开候选后点一个词，同样会留下空白面板。
        // 所以修在 commitWord 这个**所有上屏路径的汇合点**，而不是在
        // afterSegmentChosen 里单独补一句。
        //
        // 判据：上屏之后手上没有可显示的东西了，就不该继续占着「为了让候选铺开」
        // 而临时借来的整块键盘高度。
        candidateExpanded = false
        suggestions = if (thenSuggest && chinese) {
            NativeEngine.suggest(word, SUGGEST_LIMIT)
        } else {
            emptyList()
        }
        renderCandidates()
        refreshStatus()
    }

    /**
     * 补全邮箱后缀：把 `@qq.com` 直接接上去。
     *
     * 补完就把 [emailSuffixes] 清掉——后缀已经打进输入框了，再显示一遍等于
     * 让用户有机会重复补一次，补出 `123@qq.com@163.com` 这种垃圾。
     *
     * 不用 commitText 替换整段邮箱：光标前那串数字是**用户自己打的**，我们只负责
     * 补后半截。替换整段会让宿主的撤销栈、光标记录全乱（尤其是网页输入框）。
     */
    private fun commitEmailSuffix(suffix: String) {
        val ic = currentInputConnection ?: return
        ic.commitText(suffix, 1)
        emailSuffixes = emptyList()
        // 补完之后光标停在邮箱末尾，重新算一次：此时前文末尾是 "m" 不是数字，
        // 自然会算出空表（这里显式调一次是为了让状态和界面同步，别只靠清空）。
        refreshEmailSuggest()
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
        pendingSegments = emptyList()
        segmentState = null
        segmentStateCode = ""
        resetAssist()
        suggestions = NativeEngine.suggest(word, SUGGEST_LIMIT)
        renderCandidates()
        refreshStatus()
    }

    /**
     * 上屏一条快捷输入结果（「/rq」→「2026-02-14」），然后退出快捷输入态。
     *
     * 和 [commitWord] 的两处刻意不同：
     * - **不学词**：日期、时间、UUID 每次都不一样，记进「上一词→这一词」只会
     *   污染联想表（Windows 端也是这么定的）。
     * - **不联想**：上屏的是「2024-06-01」，拿它去查下一个词没有任何意义。
     */
    private fun commitQuick(word: String) {
        val ic = currentInputConnection ?: return
        ic.commitText(word, 1)
        quickMode = false
        quickWords = emptyList()
        pending.setLength(0)
        code = ""
        candidates = emptyList()
        pendingSegments = emptyList()
        segmentState = null
        segmentStateCode = ""
        suggestions = emptyList()
        // 展开态一起收（同上：上屏后没有候选可铺，不该继续占着借来的键盘高度）
        candidateExpanded = false
        // 上屏的是一串数字/符号，不能拿它当联想上下文
        prevWord = ""
        renderCandidates()
        refreshStatus()
    }

    /**
     * 当前输入框是不是**多行**文本框。
     *
     * 判据只用 `inputType` 的位标志，不看 hint / 包名——那些都是猜。
     *
     * 这个值决定回车行为的关键分支（见 [EnterSpec.decide]）：
     * 飞书/微信聊天框是「多行 + DONE」，要发换行；Settings 搜索是「单行 + DONE」，
     * 要什么都不发。两者的 action 完全相同，**只能靠这个标志区分**。
     *
     * 注意 Termux 的终端视图是 `TYPE_NULL`，这里返回 false——它走的是
     * 「没有 action → 发换行」那条独立分支，不依赖这个值。
     */
    private fun isMultilineField(): Boolean {
        if ((fieldType and android.text.InputType.TYPE_MASK_CLASS) !=
            android.text.InputType.TYPE_CLASS_TEXT
        ) {
            return false
        }
        return (fieldType and android.text.InputType.TYPE_TEXT_FLAG_MULTI_LINE) != 0
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
        resetAssist()
        candPage = 0
        quickMode = false
        quickWords = emptyList()
        pending.setLength(0)
        code = ""
        candidates = emptyList()
        pendingSegments = emptyList()
        suggestions = emptyList()
        segmentState = null
        segmentStateCode = ""
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
        // commitText 之后宿主通常会回调 onUpdateSelection，但不是所有宿主都老实
        // （网页输入框、某些自绘控件就不回调）。邮箱联想靠前文末端的数字触发，
        // 漏一次回调就等于「打完数字没反应」，所以这里主动补算一次，代价很小。
        refreshEmailSuggest()
    }

    /** 覆盖状态条文案（语音过程用）。传 null / 不传就恢复常规状态。 */
    private fun refreshStatus(override: String? = null) {
        if (override != null) {
            statusText?.text = override
            return
        }
        statusText?.text = buildString {
            // 中/英模式和当前页**不常驻显示**：底下那个「中」键已经标了当前模式，
            // 「123/符/😊」入口本身也说明了在哪一页。状态条上再写一遍「中 · 双拼」
            // 只是把候选条左边那一块占掉，用户反馈纯属多余。
            //
            // 只在有**看不见的信息**要说明时才写：正在组合的码、分段态、引擎异常。
            if (pending.isNotEmpty()) {
                append(pending)
                // 快捷输入态的码是命令码，不是拼音，别给它配一个「→实际用的码」
                if (quickMode) {
                    append(if (quickWords.isEmpty()) "  ·  没有这条命令" else "  ·  快捷输入")
                }
                // 双拼模式下码就是输入的字母，两者相同，不显示箭头；
                // 全拼模式（或双拼模式退回全拼解释）才有 `nihao→nihc`。
                if (code.isNotEmpty() && code != pending.toString()) {
                    append("→").append(code)
                }
            }
            // 分段态给一句提示，否则用户不知道候选条上一排方块是什么、怎么退出。
            val seg = segmentState
            if (seg != null) {
                if (isNotEmpty()) append("  ·  ")
                append(if (seg.focus >= 0) "选这一段的字" else "点词改某一段")
            }
            if (!NativeEngine.available) {
                if (isNotEmpty()) append("  ·  ")
                append("引擎未加载")
            }
        }
    }

    /**
     * 键盘收起：短信监听跟着摘掉。
     *
     * 系统和用户都看得见「这个输入法在键盘开着的时候瞄了一眼短信」——键盘一收就停，
     * 不留后台监听。
     */
    override fun onFinishInputView(finishingInput: Boolean) {
        stopSmsObserver()
        super.onFinishInputView(finishingInput)
    }

    override fun onFinishInput() {
        stopSmsObserver()
        prevWord = ""
        clearComposing()
        super.onFinishInput()
    }

    private fun dp(v: Float): Int = (v * resources.displayMetrics.density + 0.5f).toInt()

    /**
     * 按当前屏幕宽度和系统字号，重算展开态每行几列。
     *
     * ## 为什么必须自适应，不能写死
     *
     * 展开候选的格子是「屏宽 ÷ 列数」等分出来的。列数写死 6 时，窄屏（360dp）
     * 每格只有 57dp，扣掉格内 padding 剩 45dp，三字词「指挥官」需要 60dp
     * → 被 ellipsize 截成「指…」，等于候选废了。写死 4 又会在平板/大屏上
     * 每格宽到离谱、一行只放 4 个，浪费一半横向空间。
     *
     * 换手机、折叠屏展开/折叠、用户把系统字号调大 —— 这几种情况屏宽或字宽都会变，
     * 所以列数必须在**每次创建输入视图**和**配置变化**时重算。
     *
     * 判据见 [CandidateGrid.columnsFor]：每格至少放得下三字词。
     */
    private fun updateColumns() {
        val dm = resources.displayMetrics
        val widthDp = (dm.widthPixels / dm.density).toInt()
        // 系统字号缩放：用户调大字号时汉字实际占宽同步变大，列数要跟着减
        val fontScale = if (dm.density > 0f) dm.scaledDensity / dm.density else 1f
        val cols = CandidateGrid.columnsFor(widthDp, fontScale)
        if (cols != CandidateGrid.PER_ROW) {
            CandidateGrid.PER_ROW = cols
            // 列数变了，已铺好的网格全部作废，下一次 renderCandidates 重建
            expandedColumn?.removeAllViews()
            candidateExpanded = false
        }
    }

    /**
     * 配置变化（换屏、旋转、系统字号改、深色模式切换）后重算列数并重绘。
     *
     * 不重算的后果：用户在设置里把字号从「小」调到「超大」，回来打字时列数
     * 还是按旧字号算的，三字词立刻被截断——而且这个 bug 会一直持续到输入法
     * 进程被重启。
     */
    override fun onConfigurationChanged(newConfig: android.content.res.Configuration) {
        super.onConfigurationChanged(newConfig)
        val before = CandidateGrid.PER_ROW
        updateColumns()
        if (before != CandidateGrid.PER_ROW || candidateExpanded) {
            renderCandidates()
        }
    }


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

        /** 顶部那一行的高度（dp）。状态栏和候选条轮流用，所以取两者较高的那个。 */
        private const val TOP_SLOT_DP = 44f

        /**
         * 候选条每一行的高度（dp）。展开态是「行数 × 这个值」。
         *
         * 固定 40dp 是按 20sp 候选字算的（字高约 27dp + 上下留白），经 `dp()`
         * 按屏幕密度缩放，不是像素死值。
         *
         * **已知边界**：它不随系统字号（fontScale）放大。用户把字号调到 2.0 倍时
         * 20sp 实际占 40dp，会贴满整行高度。没改成自适应是因为 20 处使用点都在
         * 高度计算里（行高×行数、容器高度、ScrollView 可视区），一动就要重新
         * 验证整套纵向布局；而列数自适应（[CandidateGrid.columnsFor]）已经解决了
         * 「字被截断/折行」这个用户实际遇到的问题，大字号下的贴边属于观感问题。
         */
        private const val CANDIDATE_ROW_DP = 40f

        /**
         * 控件描边宽度。
         *
         * 1dp 而不是 1.5dp：440dpi 屏上 1dp≈2.75px、1.5dp≈4px，4px 的框在
         * 一排候选文字里显得笨重，用户反馈「那几个功能按钮边框有点粗」。
         * 1dp 仍然清晰可辨（配合 chipBg 浅底），但不会抢候选词的视觉重心。
         *
         * **不要低于 1dp**：再细在低密度屏上会被抗锯齿吃掉，框又会变回「看不见」。
         */
        private const val CHIP_STROKE_DP = 1f

        /** 数字选词行的高度（dp）。恒定占位，不随候选有无伸缩。 */
        private const val NUM_ROW_DP = 36f

        /**
         * 一次向引擎要多少个候选。**不设上限。**
         *
         * ## 为什么是 Int.MAX_VALUE
         *
         * 用户明确要求「要无限」。候选条的意义就是「把同码的字都摊开让人挑」，
         * 在数据层截断等于替用户决定「这个字你不该看到」——用户会以为词库里没有。
         *
         * 这里原本是 200，注释理由是「再大就要排序几万条，输入会有明显延迟」。
         * **那是凭空推测，实测数据不支持**（host x86_64 release）：
         *
         * ```
         * 单键 l  : 565 条  0.355ms      （200 限制时 0.111ms）
         * 单键 y  : 734 条  0.573ms
         * 单键 yi : 845 条  0.200ms
         * 双键 lq : 154 条  0.073ms
         * ```
         *
         * 最坏 0.573ms，离「能感觉到」差一个数量级；而且候选最多的码也只有
         * 845 条——单键候选是有限的（词库就那么些同码词），**根本不存在几万条**。
         * 真机 ARM 慢几倍也在 1~3ms，无感。
         *
         * 数量控制交给 UI（收起态横滑 / 展开态滚动），不该在数据层偷偷截断。
         */
        private const val CAND_FETCH = Int.MAX_VALUE

        /**
         * 分段选字时**每个字**给几个候选。
         *
         * ## 为什么不能只给 9 个
         *
         * 原值 9，理由是「双拼两码同音字通常几十个，9 个够用」。**这个判断是错的**：
         * 逐字选词是用户自造词的唯一路径，用户要的那个字如果排在 10 名以后
         * （生僻字、人名用字、方言字），就**永远选不到**——候选条横滑到底也只有 9 个，
         * 用户会以为输入法里没这个字。
         *
         * 和 [CAND_FETCH] 取同一个值：两条路径都是「把同码候选摊开让人挑」，
         * 没有任何理由一条给 200 一条给 9。想限制数量的话，UI 层横滑/滚动已经
         * 在承担这个职责了，不该在数据层偷偷截断。
         */
        private const val SEG_CAND_LIMIT = CAND_FETCH

        /**
         * 快速命令（`/rq`、`/sj` 之类）的预览条数。
         *
         * 和分段选字**必须分开**：命令只有十来条，给 200 会让预览条长得没法看，
         * 而且命令是「点一下就上屏」的，多了反而增加误触。
         */
        private const val QUICK_CAND_LIMIT = 9
    }
}
