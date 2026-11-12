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
import com.feihe.ime.suggest.CandidateBarPolicy
import com.feihe.ime.suggest.EmailSuggest
import com.feihe.ime.suggest.HostContext
import com.feihe.ime.voice.VoiceEngine
import com.feihe.ime.voice.VoiceRecorder
import com.feihe.ime.keyboard.Backspace
import com.feihe.ime.keyboard.CandidateGrid
import com.feihe.ime.keyboard.Key
import com.feihe.ime.keyboard.KeyKind
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

    /** 一屏候选上限（5/7/9）。查引擎和渲染共用同一个数，避免两处不一致。 */
    private var candidateLimit = CandidateBarPolicy.DEFAULT_LIMIT

    private var statusText: TextView? = null
    private var micButton: TextView? = null
    private var statusBar: LinearLayout? = null
    private var voice: VoiceRecorder? = null

    /** 当前语音状态。麦克风按钮是两态开关，必须知道「现在录没录」才能决定点了做什么。 */
    private var voiceState = VoiceRecorder.State.IDLE

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

    /** 候选翻到第几页（0 起）。每页 [candidateLimit] 个，见 [visibleCandidates]。 */
    private var candPage = 0

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

    /** 当前输入框的 inputType / hint。判断「像不像验证码框」用，见 [OtpCode]。 */
    private var fieldType = 0
    private var fieldHint: CharSequence? = null

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
                commitText(text)
                forgetContext()
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
        candidateLimit = UiPrefs.candidateLimit(this)
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
        val gear = TextView(this).apply {
            text = "⚙"
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 18f)
            gravity = Gravity.CENTER
            setTextColor(statusPalette.sub)
            setPadding(dp(14f), dp(6f), dp(14f), dp(6f))
            contentDescription = "输入法设置"
            setOnClickListener { openSettings() }
        }

        // 剪贴板历史入口。放在状态条上而不是候选条里：候选条只在有候选项时才有意义，
        // 而剪贴板随时可能要用（复制了链接、地址、验证码之后）。
        clipButton = TextView(this).apply {
            text = "📋"
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 17f)
            gravity = Gravity.CENTER
            setTextColor(statusPalette.sub)
            setPadding(dp(12f), dp(6f), dp(12f), dp(6f))
            contentDescription = "剪贴板历史"
            setOnClickListener { toggleClipboard() }
        }

        // 麦克风：**单击开始、再单击结束**。
        //
        // 以前是按住说话（OnTouch DOWN 开录 / UP 停录），但状态条上的按钮离
        // 屏幕底边很近，长按容易滑出去触发 ACTION_CANCEL；而且用户腾不出另一只手
        // 去点「结束」。改成两态开关后，录多久都行，状态条会显示「正在录音」。
        micButton = TextView(this).apply {
            text = "🎤"
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 17f)
            gravity = Gravity.CENTER
            setPadding(dp(12f), dp(6f), dp(12f), dp(6f))
            contentDescription = "语音输入"
            visibility = if (voiceEnabled) View.VISIBLE else View.GONE
            setOnClickListener { toggleVoice() }
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
            addView(
                expandedColumn,
                ViewGroup.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    ViewGroup.LayoutParams.WRAP_CONTENT,
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
            gravity = Gravity.CENTER
            visibility = View.GONE
            setPadding(dp(8f), dp(6f), dp(8f), dp(6f))
            setOnClickListener {
                candidateExpanded = false
                renderCandidates()
                refreshStatus()
            }
            addView(TextView(this@FeiheIme).apply {
                text = "▴ 收起候选"
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 13f)
                gravity = Gravity.CENTER
                setTextColor(Palette.of(this@FeiheIme).sub)
            })
        }
        root.addView(
            collapseBar,
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.WRAP_CONTENT,
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
                KeyboardPages.rowHeightDp(target),
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
     * 点了英文单词建议：用它**替换**输入框里未确认的那串。
     *
     * `commitText` 会把当前组合区整体替换掉，正好是「把 hel 换成 hello」
     * 这个语义。绝不能用 append——那会得到 helhello。
     */
    private fun pickEnglishWord(word: String) {
        val ic = currentInputConnection ?: return
        ic.commitText(word, 1)
        pending.setLength(0)
        englishWords = emptyList()
        englishSuggest?.query("")
        refreshCandidates()
        refreshStatus()
        forgetContext()
    }

    private fun onKey(key: Key) {
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
                // **英文态：字母进组合态（带下划线），不是直接上屏。**
                //
                // 参照豆包输入法的行为（用户实测那套用着习惯）：
                //   - 打字母 → 进输入框但**带下划线**（未确认状态，setComposingText）
                //   - 候选栏同时给出英文单词建议
                //   - 点候选 → 用它替换掉未确认的那串
                //   - 不点候选、直接空格/回车 → 原文确认上屏
                //
                // 为什么不能像拼音那样攒在 pending 里只显示在候选条：英文单词
                // 长度不定，用户需要「边打边看到自己在打什么」。组合态让字母
                // 直接出现在光标处、又保留可被候选替换的可能，这才是对的模型。
                //
                // 注意这里**不再清空 pending**——英文也用它攒字母，只是渲染
                // 走组合态那条路（见 pushComposingEnglish）。
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
                        if (pending.isEmpty()) {
                            // 组合区空了：必须用空串再 set 一次，宿主才会撤掉下划线
                            currentInputConnection?.setComposingText("", 1)
                            englishWords = emptyList()
                            englishSuggest?.query("")
                            refreshCandidates()
                            refreshStatus()
                        } else {
                            pushComposingEnglish()
                        }
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
                        assistActive = false
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
                    // 没有分段数据时（引擎没算出可切分）才退回删字母，保证不会卡死。
                    if (!backspaceIntoSegments()) {
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
                // 英文组合态：回车先确认原文，再换行。和空格同一条规则——
                // 用户打了半个词直接回车，本意是「就这样，换行」，不是「丢掉」。
                if (!chinese && pending.isNotEmpty()) {
                    commitEnglishComposing()
                }
                if (!commitSegmentIfActive()) commitPending()
                commitText("\n")
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
     * 英文组合态：字母进输入框并**带下划线**（未确认），同时查单词建议。
     *
     * 这就是豆包输入法的英文行为：输入框里能看到自己在打什么，
     * 下面给单词候选；点候选就替换，按空格/回车就确认原文。
     *
     * `setComposingText` 正是「未确认」的官方语义：宿主会画下划线，
     * 之后的 `commitText` 或再次 `setComposingText` 都不会留下残影。
     */
    private fun pushComposingEnglish() {
        val ic = currentInputConnection ?: return
        ic.setComposingText(pending.toString(), 1)
        refreshStatus()
        // 建议是异步的（跨进程问系统拼写检查器），回来再渲染
        englishSuggest?.query(pending.toString())
    }

    /** 英文建议回来了：存起来并只重画候选条，不动输入框。 */
    private fun onEnglishSuggest(prefix: String, words: List<String>) {
        // 用户在等待期间又改了词 → 这批结果作废
        if (!chinese && prefix != pending.toString()) return
        englishWords = words
        renderCandidates()
    }

    /**
     * 结束英文组合态：把当前未确认的字母变成正式文本。
     *
     * `commitText` 会把组合区替换成同样的内容，效果就是「确认」——
     * 下划线消失，字符留下。空格键和回车键都走这里。
     */
    private fun commitEnglishComposing(): Boolean {
        if (chinese || pending.isEmpty()) return false
        val ic = currentInputConnection ?: return false
        ic.commitText(pending.toString(), 1)
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

    private fun resetAssist() {
        assistActive = false
        assistKeys.setLength(0)
        assistWords = emptyList()
    }

    /** 点「形」：进/出形码态。进出都清空已打的形码——上一次的筛选不该带过来。 */
    private fun toggleAssist() {
        if (!assistActive && !assistAvailable()) return
        assistActive = !assistActive
        assistKeys.setLength(0)
        assistWords = emptyList()
        refreshAssist()
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
        // 形码一变，候选整批换掉：回到第一页
        candPage = 0
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
            // 只打了 `/` 时 key 是空串，引擎返回全部命令的当前值——正好当预览。
            quickWords = NativeEngine.quickCandidates(
                pending.toString().removePrefix(NativeEngine.quickPrefix()),
                CandidateBarPolicy.candidateSlots(candidateLimit, false),
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
            // 新的一次查询 = 从第一页开始看
            candPage = 0
        }
        refreshSegmentMode()
        renderCandidates()
    }

    /**
     * 要不要进「逐段选择」态。
     *
     * 判据**完全来自引擎**：`pendingSegments` 为空就是普通候选——它是这次查询里
     * core 顺手算好带回来的（`lookup_input.segments`），不是壳猜的。以前这里会再调
     * 一次 JNI 重算分段，长串输入时同一段 DP 每次按键跑两遍。
     */
    private fun refreshSegmentMode() {
        segmentState = if (chinese) {
            // 带上每段的码：上屏时要靠它把「码 → 词」记进自造词表。
            SegmentState(
                segments = pendingSegments.map { it.words },
                codes = pendingSegments.map { it.code },
            ).takeIf { it.active }
        } else {
            null
        }
    }

    /**
     * 分段态下按了收尾键（空格/回车/标点/字符）：先把整句上屏。
     *
     * **这里必须学一次词**：逐字选词是用户造词的主要路径（打 feih → 选「飞」→
     * 选「和」），而它全程不经过 [commitWord]，所以以前这条路一个字都没记住。
     * 用户体感就是「自组词功能依然不生效」。
     */
    private fun commitSegmentIfActive(): Boolean {
        val state = segmentState ?: return false
        val sentence = state.sentence()
        commitWord(sentence, thenSuggest = false)
        // 记「码 → 词」。段码是双拼码（feih），但 phrase 表按**全拼**索引，
        // 直接存双拼码下次查不到——必须先转成 fei he（见 pinyinOf 的注释）。
        NativeEngine.learn(state.sentenceCode(), sentence)
        rememberPhrase(sentence, state)
        return true
    }

    /**
     * 把逐段拼出来的整句记成自造词。
     *
     * 优先用段码反查全拼（准确，尊重当前方案）；段码缺失时退回按字反查。
     * 两条都得拿到全拼才能落库，拿不到就放弃——宁可这次不记，也别存一条
     * 永远捞不出来的死数据。
     */
    private fun rememberPhrase(sentence: String, state: SegmentState) {
        if (sentence.length < 2) return
        // 词库里已经有 → 不是自造词
        if (NativeEngine.pinyinOf(state.sentenceCode(), sentence).isNotEmpty()) return

        // 段码可用：直接用查出来的全拼
        val fromSeg = state.sentenceCode().takeIf { it.isNotEmpty() }
            ?.let { NativeEngine.pinyinOf(it, sentence) }
            ?.takeIf { it.isNotEmpty() }
        if (fromSeg != null) {
            NativeEngine.learnPhrase(sentence, fromSeg)
            return
        }
        // 退化：逐字反查全拼
        rememberPhraseIfNew("", sentence)
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
            // 分段态的 chip 带选中底色、还有「‹ 返回」这种特殊项，画法不同且低频，
            // 直接整条重画（它不占键盘高度，重建的抖动没人看得见）。
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
            val exp = expandedColumn ?: return
            candidateScroll?.visibility = View.GONE
            // 可见性改在 ScrollView 上：expandedColumn 是它的内容，
            // 让内容可见而容器 GONE 等于什么都不显示。
            expandedScroll?.visibility = View.VISIBLE
            renderExpandedRows(exp, window)
            updateTopSlot()
            return
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
        // 注意要排在 assistChip/otp 之后：那两个是「形码筛选」「验证码提示」这种
        // 状态位，必须压在最左，箭头挤到它们前面会让人以为筛选按钮没了。
        if (assistChip) nextSlot()?.let { bindAssistView(it) }
        if (otp != null) nextSlot()?.let { bindOtpView(it, otp) }
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
     * 不加数字编号——触屏上直接用手指点，编号是给键盘选词的电脑用的。见
     * [candidateExpanded] 的注释。
     */
    private fun renderExpandedRows(col: LinearLayout, window: List<String>) {
        val rows = CandidateGrid.rows(window.size, expandedRows())
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
            val want = rowH * rows
            (sv.layoutParams as? LinearLayout.LayoutParams)?.let { lp ->
                if (lp.height != want || lp.weight != 0f) {
                    lp.height = want
                    lp.weight = 0f
                    sv.layoutParams = lp
                }
            }
            // 内容高度也钉死：让 ScrollView 知道内容有多高，才滚得动
            (col.layoutParams as? ViewGroup.LayoutParams)?.let { lp ->
                if (lp.height != want) {
                    lp.height = want
                    col.layoutParams = lp
                }
            }
            sv.scrollTo(0, 0)
        }
        window.forEachIndexed { idx, word ->
            val row = col.getChildAt(idx / CandidateGrid.PER_ROW) as LinearLayout
            val slot = idx % CandidateGrid.PER_ROW
            val v = row.getChildAt(slot) as? TextView ?: TextView(this).also {
                row.addView(it, LinearLayout.LayoutParams(0, dp(CANDIDATE_ROW_DP), 1f))
            }
            bindCandidateView(v, word, idx == 0, code)
        }
        // 每行末尾多余的格子要删掉：上一页这行可能有 6 个，这一页只有 2 个
        for (r in 0 until rows) {
            val row = col.getChildAt(r) as LinearLayout
            val keep = minOf(CandidateGrid.PER_ROW, window.size - r * CandidateGrid.PER_ROW)
            while (row.childCount > keep) row.removeViewAt(row.childCount - 1)
        }
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
        v.background = null
        v.text = if (candidateExpanded) "▴" else "▾"
        v.setTextSize(TypedValue.COMPLEX_UNIT_SP, 14f)
        v.setPadding(dp(8f), 0, dp(8f), 0)
        v.setTextColor(Palette.of(this).sub)
        v.contentDescription = if (candidateExpanded) "收起候选" else "展开候选，显示更多"
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
     * 展开态实际能放几行。
     *
     * 展开时候选区会吃掉整个键盘的高度（键盘被 GONE 掉），所以行数不能写死——
     * 写死 3 行的话，剩下那片空白就白白浪费了。这里按输入法视图的**真实高度**
     * 换算：屏幕高度 - 状态条 - （上下一点留白），再除以一行的高度。
     *
     * 拿不到高度（还没 measure，比如刚 onCreateInputView 就 refresh）时退回
     * [CandidateGrid.MAX_ROWS]，保证算出来的永远是个正数。
     */
    private fun expandedRows(): Int {
        val rowH = dp(CANDIDATE_ROW_DP)
        if (rowH <= 0) return CandidateGrid.MAX_ROWS
        // 可用高度 = 输入法视图能拿到的总高度 − 状态条 − 收起条。
        // 展开时键盘和数字行都是 GONE，不再占位，所以剩下的全给候选。
        //
        // 拿 root 的实际高度最准，但它要 measure 完才有值；没值时退回屏幕高度
        // 按键盘占比估算（键盘一般占屏幕 45%~55%）。
        val rootH = rootViewHeight()
        val screenH = resources.displayMetrics.heightPixels
        val total = if (rootH > 0) rootH else (screenH * 0.55f).toInt()
        var chrome = 0
        statusBar?.let { if (it.height > 0) chrome += it.height else chrome += dp(34f) }
        chrome += dp(30f) // 收起条自己
        val usable = total - chrome - rowH // 再扣一行余量，免得最后一行被裁半截
        if (usable <= 0) return CandidateGrid.MAX_ROWS
        return (usable / rowH).coerceIn(1, 8)
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
     * 一页几个候选。
     *
     * 收起态 = 设置里的「候选个数」（5/7/9）；展开态 = 行 × 列（见 [CandidateGrid]）。
     * 两个模式都从这里取，保证展开/收起切换时窗口计算一致。
     */
    private fun pageSize(): Int =
        CandidateGrid.pageSize(
            candidates.size,
            candidateExpanded,
            candidateLimit.coerceAtLeast(1),
            expandedRows(),
        )

    /**
     * 当前页的候选。候选一共 [CAND_FETCH] 个，靠 ‹ › 翻页，不靠横滑。
     *
     * 页码越界时夹回最后一页（不是返回空表），见 [CandidateGrid.window]。
     */
    private fun visibleCandidates(): List<String> =
        CandidateGrid.window(
            candidates,
            candPage,
            candidates.size,
            candidateExpanded,
            candidateLimit.coerceAtLeast(1),
            expandedRows(),
        )

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
        v.setPadding(dp(14f), 0, dp(14f), 0)
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
     * 「形」按钮。**文案里必须带上已打的形码**（`形 tl`）：形码是看不见的状态，
     * 只亮一个按钮的话，用户不知道刚才那两个键到底吃进去没有。
     */
    private fun bindAssistView(v: TextView) {
        v.background = null
        val p = Palette.of(this)
        v.text = when {
            !assistActive -> "形"
            assistKeys.isEmpty() -> "形 _"
            else -> "形 $assistKeys"
        }
        v.setTextSize(TypedValue.COMPLEX_UNIT_SP, 15f)
        v.setPadding(dp(10f), 0, dp(10f), 0)
        v.setTextColor(if (assistActive) p.accent else p.sub)
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
                                segmentState = next
                                // 换段清形码；已经是最后一段就退出形码态
                                assistKeys.setLength(0)
                                if (next.focus < 0) assistActive = false
                                updateAssistWords()
                                renderCandidates()
                            } else {
                                // 未筛选时下标就在当前段里，直接换算回去
                                val idx = state.focusedWords().indexOf(word)
                                segmentState = state.choose(idx)
                                renderCandidates()
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
        // 引擎没给分段数据就退不进去（长串没打完、或词库里切不开）
        val next = SegmentState(pendingSegments.map { it.words }).takeIf { it.active }
            ?: return false
        segmentState = next
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
        // 形码跟着这次组合一起结束：留着的话下一个词的候选会被上一词的形码筛空
        resetAssist()
        candPage = 0
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
        suggestions = emptyList()
        // 上屏的是一串数字/符号，不能拿它当联想上下文
        prevWord = ""
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

/** 候选条每一行的高度。展开态是「行数 × 这个值」。 */
private const val CANDIDATE_ROW_DP = 40f

        /** 数字选词行的高度（dp）。恒定占位，不随候选有无伸缩。 */
        private const val NUM_ROW_DP = 36f

        /** 一次向引擎要多少个候选：够翻五页（每页最多 9 个）。 */
        private const val CAND_FETCH = 45
    }
}
