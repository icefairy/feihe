package com.feihe.ime.keyboard

import android.content.Context
import android.graphics.drawable.GradientDrawable
import android.util.TypedValue
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
import com.feihe.ime.Palette
import com.feihe.ime.UiPrefs

/**
 * 把 [KeyboardPages] 的行数据画成一棵 View 树。
 *
 * **颜色一律取自 [Palette]**，这个类里不允许再出现字面量色值——深色模式下
 * 漏一处就是一块刺眼的白底（或黑底黑字）。深浅两套由 Palette 按系统夜间模式选。
 */
object KeyboardView {

    private const val KEY_HEIGHT_DP = 46f
    private const val GAP_DP = 3f

/**
 * 空格键按住多久进语音（毫秒）。
 *
 * ## 为什么从 220ms 提到 400ms
 *
 * 220ms 太短了：普通人点一下键的接触时长是 150~250ms（按下到抬起），
 * 落在 220ms 两侧，**同一个手势有时当空格、有时当语音**。用户体感就是
 * 「空格单击完全失效了」——其实是按钮把每一次稍慢的点击都吃成了长按。
 *
 * 400ms 是明确的分界：正常点击达不到，想说一句话也无需干等。
 * 系统长按判定 500ms 太迟钝，所以不用系统的。
 */
private const val VOICE_HOLD_MS = 400L

/**
 * 手指移动超过这个距离（dp）就取消语音判定。
 *
 * 打字时手指常带一点滑动，尤其空格这种大键。不判移动的话「手一歪」
 * 也会进语音，而用户本意是打空格。
 */
private const val VOICE_CANCEL_SLOP_DP = 24f

    /**
     * 上档键的 tag。
     *
     * 上档状态变了**不重建整个键盘**：重建会闪一下、候选条也跟着抖，而且用户只是
     * 按了一下 ⇧。改成按 tag 找到那颗键、就地换配色。
     */
    const val SHIFT_TAG = "feihe.shift"

    /**
     * 中/英键的 tag。切模式时**只改这一颗键的字**，不重建键盘——重建会闪，
     * 而且用户刚按下的位置会短暂消失。
     */
    const val MODE_TAG = "feihe.mode"

    fun build(
        ctx: Context,
        rows: List<List<Key>>,
        shift: ShiftState = ShiftState.OFF,
        rowHeightDp: Float = KEY_HEIGHT_DP,
        onLongKey: (Key) -> Unit = {},
        onVoiceUp: () -> Unit = {},
        /** 形码方案：非空表示当前处于形码态，按键要显示形码提示（见 [makeAssistKey]）。 */
        assistMode: String? = null,
        onKey: (Key) -> Unit,
    ): View {
        val p = Palette.of(ctx)
        val root = LinearLayout(ctx).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(p.keyboardBg)
            // 左右只留 2dp：原先 3dp 在 1080p 上是 9px，看着明显的白边，
            // 而字母键本身已经被 margin 收缩过了，两侧不需要再额外让位。
            setPadding(dp(ctx, 2f), dp(ctx, 4f), dp(ctx, 2f), dp(ctx, 4f))
        }

        for (row in rows) {
            val rowView = LinearLayout(ctx).apply {
                orientation = LinearLayout.HORIZONTAL
                // weightSum 必须显式给：TextView 的 layout_weight 只有在父容器
                // 有 weightSum 或其它子项也用 weight 时才按比例分，否则会按 1:1 摊，
                // 空格键就和字母键一样宽了。
                weightSum = row.sumOf { it.weight.toDouble() }.toFloat()
            }
            val lp = LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                dp(ctx, rowHeightDp),
            ).apply { bottomMargin = dp(ctx, GAP_DP) }

            for (key in row) {
                // 形码态：字母键换成「字母小字 + 形码大字」的双层布局，
                // 让用户照着键面点，不用背部首表。
                val hint = if (assistMode != null && key.kind == KeyKind.CHAR) {
                    AssistKeyHints.hintFor(key.code.firstOrNull() ?: ' ', assistMode)
                } else ""
                val v = if (hint.isNotEmpty()) {
                    makeAssistKey(ctx, key, hint, assistMode, onLongKey, onKey)
                } else {
                    makeKey(ctx, key, shift, onLongKey, onVoiceUp, onKey)
                }
                rowView.addView(v, keyParams(ctx, key))
            }
            root.addView(rowView, lp)
        }
        return root
    }

    /**
     * 形码态下的按键：上排小字字母（淡化），下排大字形码提示。
     *
     * ## 为什么不是「隐藏字母、只显示部首」
     *
     * 用户明确要求「原来按键的标点与字母可以暂时隐藏」，但**字母必须留一点痕迹**：
     * 形码本质还是按字母键，完全抹掉字母后，用户按下 `s` 却看不到 `s`，
     * 会怀疑自己点错了键。所以做成「字母缩小淡化 + 部首放大」的主次关系，
     * 既突出形码又不让人失去坐标感。
     *
     * ## 为什么用 FrameLayout 而不是两个 LinearLayout 子项
     *
     * 键高只有 46dp，两行文字垂直居中分布时，中间不留空隙会让两行挤在一起。
     * FrameLayout + 上下两个 gravity 能各自贴边，中间的留白由键高自然分配，
     * 并且**不改变按键本身的点击区域**（整个 FrameLayout 就是一个可点区域）。
     *
     * ## 长按提示
     *
     * 键面只放得下两个字（`s` → 手水），但 `s` 实际对应 25 个部首。长按弹气泡
     * 把完整列表摊开——「用多了自然会」，但在会之前得有一条查得到的路。
     */
    private fun makeAssistKey(
        ctx: Context,
        key: Key,
        hint: String,
        mode: String?,
        onLongKey: (Key) -> Unit,
        onKey: (Key) -> Unit,
    ): View {
        val p = Palette.of(ctx)
        val sizeScale = UiPrefs.keyScale(ctx)
        val base = keyTextSizeSp(key) * sizeScale

        // 上层：字母，缩小淡化，只作方位参考。形码态键面信息量比普通键大
        // （字母 + 两个部件），两行都按普通键字号会挤，所以字母压到 45%。
        val letter = TextView(ctx).apply {
            text = key.label
            gravity = Gravity.CENTER
            setTextSize(TypedValue.COMPLEX_UNIT_SP, base * ASSIST_LETTER_SCALE)
            setTextColor(p.fnText)
            alpha = 0.55f
            isClickable = false
            isFocusable = false
            layoutParams = FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT,
                FrameLayout.LayoutParams.WRAP_CONTENT,
            ).apply { gravity = Gravity.TOP or Gravity.CENTER_HORIZONTAL }
        }
        // 下层：形码提示。用户原话「可以用小一些的字号」——0.82 太大，
        // 两个汉字摊在 36dp 键宽里发胀。压到 0.60（20sp 基值 → 12sp），
        // 仍清晰但不抢字母、不撑键帽。
        val hintView = TextView(ctx).apply {
            text = hint
            gravity = Gravity.CENTER
            setTextSize(TypedValue.COMPLEX_UNIT_SP, base * ASSIST_HINT_SCALE)
            setTextColor(p.keyText)
            isClickable = false
            isFocusable = false
            // 部件提示可能有 1~2 个字，统一单行显示，不给它换行把键撑高的机会
            isSingleLine = true
            layoutParams = FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT,
                FrameLayout.LayoutParams.WRAP_CONTENT,
            ).apply { gravity = Gravity.BOTTOM or Gravity.CENTER_HORIZONTAL }
        }

        val box = FrameLayout(ctx).apply {
            // 背景跟普通键一致：形码态只是「换了键上的字」，不该让整个键盘变样
            background = keyBackground(ctx, key, ShiftState.OFF)
            isClickable = true
            setPadding(dp(ctx, 2f), dp(ctx, 3f), dp(ctx, 2f), dp(ctx, 3f))
            contentDescription = "${key.label}，形码 ${hint}"
            addView(letter)
            addView(hintView)
            setOnClickListener { onKey(key) }
            // 长按：走通用的 onLongKey（去掉键/切页等），同时弹形码完整提示。
            // 两个动作不冲突——长按弹气泡是「看看这个键有什么」，不改变输入。
            setOnLongClickListener {
                showAssistBubble(ctx, this, key, mode)
                onLongKey(key)
                true
            }
        }
        return box
    }

    /**
     * 长按某个字母键时弹出该键的**全部**部首/部件。
     *
     * 用 PopupWindow 而不是 Toast：气泡要贴着被按的那颗键、并且用户想看清楚时
     * 可以盯着不放（Toast 会自己消失）。
     *
     * 锚点是**被长按的那颗键自己**（`anchor` 传进来），不是 DecorView：
     * 输入法窗口的 DecorView 覆盖整个键盘，拿它当锚点会把气泡弹到屏幕角落。
     */
    private fun showAssistBubble(ctx: Context, anchor: View, key: Key, mode: String?) {
        val all = AssistKeyHints.fullHint(key.code.firstOrNull() ?: ' ', mode ?: "")
        if (all.isEmpty()) return
        val p = Palette.of(ctx)
        val title = if (mode == "xiaohe") "小鹤音形" else "自然码"
        val tv = TextView(ctx).apply {
            text = "${key.label.uppercase()} · $title\n$all"
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 13f)
            setTextColor(p.keyText)
            setPadding(dp(ctx, 12f), dp(ctx, 8f), dp(ctx, 12f), dp(ctx, 8f))
            // 部首列表可能很长（s 键 25 个），限制宽度让它自动换行而不是拉成一长条
            maxWidth = dp(ctx, 240f)
            background = GradientDrawable().apply {
                setColor(p.keyboardBg)
                setStroke(dp(ctx, 1f), p.chipStroke)
                cornerRadius = dp(ctx, 8f).toFloat()
            }
        }
        val pop = android.widget.PopupWindow(
            tv,
            ViewGroup.LayoutParams.WRAP_CONTENT,
            ViewGroup.LayoutParams.WRAP_CONTENT,
            true,
        ).apply {
            isOutsideTouchable = true
            // 点气泡外任意位置关闭——输入法窗口里没有别的关闭途径
            setBackgroundDrawable(android.graphics.drawable.ColorDrawable(0x00000000))
        }
        // 向上弹（负偏移）：气泡出现在按键上方，不挡住用户正在看的键面
        runCatching { pop.showAsDropDown(anchor, 0, -dp(ctx, 72f)) }
            .onFailure { pop.dismiss() } // 拿不到锚点就不弹，绝不因为提示功能崩掉输入法
    }

    private fun keyParams(ctx: Context, key: Key) = LinearLayout.LayoutParams(
        0,
        ViewGroup.LayoutParams.MATCH_PARENT,
        key.weight,
    ).apply {
        // 键与键之间留缝：贴着画的话手指按在交界处会分不清按到了哪个
        leftMargin = dp(ctx, GAP_DP / 2)
        rightMargin = dp(ctx, GAP_DP / 2)
    }

    /**
     * 上档状态变化时调用：只改 ⇧ 那一颗键的底色、字色和**标签**。
     *
     * 字母键的标签本来就是大写的（手机键盘的通行做法），所以**状态只能靠 ⇧ 自己
     * 表示**——不染色的用户根本分不清刚才那一下有没有点上。锁定态再换一个字符
     * `⇪`，否则「一次性」和「锁定」看起来一模一样，用户不知道为什么打完没落下来。
     */
    fun applyShiftState(root: View?, state: ShiftState) {
        val v = root?.findViewWithTag<TextView>(SHIFT_TAG) ?: return
        val p = Palette.of(root.context)
        v.text = shiftLabel(state)
        v.background = keyBackground(root.context, Key("⇧", KeyKind.SHIFT), state)
        v.setTextColor(if (state != ShiftState.OFF) p.onAccent else p.fnText)
        v.contentDescription = shiftAccessibilityLabel(state)
    }

    /**
     * 中/英键跟着当前模式走：中文态显示「中」，英文态显示「英」并染成强调色。
     *
     * 以前切了模式这颗键还写着「中」，用户按完看一眼会以为没生效——实际已经切了。
     */
    fun applyModeState(root: View?, chinese: Boolean) {
        val v = root?.findViewWithTag<TextView>(MODE_TAG) ?: return
        val p = Palette.of(root.context)
        v.text = if (chinese) "中" else "英"
        v.setTextColor(if (chinese) p.fnText else p.accent)
        v.contentDescription = if (chinese) "中英切换，当前中文" else "中英切换，当前英文"
    }

    /** ⇧ / ⇪：一次性与锁定必须长得不一样。 */
    private fun shiftLabel(state: ShiftState): String =
        if (state == ShiftState.LOCKED) "⇪" else "⇧"

    private fun shiftAccessibilityLabel(state: ShiftState): String = when (state) {
        ShiftState.OFF -> "上档（点一下，下一个字母大写）"
        ShiftState.ONCE -> "上档已开（下一个字母大写）"
        ShiftState.LOCKED -> "大写锁定已开（点一下解除）"
    }

    private fun makeKey(
        ctx: Context,
        key: Key,
        shift: ShiftState,
        onLongKey: (Key) -> Unit,
        onVoiceUp: () -> Unit,
        onKey: (Key) -> Unit,
    ): View {
        val p = Palette.of(ctx)
        val lit = key.kind == KeyKind.SHIFT && shift != ShiftState.OFF
        // 空格键长按说话的状态。每建一个键就重置：键盘页切换后旧的判定必须作废，
        // 否则「上一页按到一半的长按」会误触发新页面上的语音。
        var voiceHoldTriggered = false
        // 长按判定是否还有效（手指没滑开）。滑开就置 false 并撤销定时器。
        var held = false
        val voiceHoldRunnable = Runnable {
            voiceHoldTriggered = true
            onLongKey(key)
        }
        val view = TextView(ctx).apply {
            text = when {
                key.kind == KeyKind.SHIFT -> shiftLabel(shift)
                // 空格键带一个麦克风小字，提示「按住可以说话」。
                // 用 ⌾ 而不是 🎤：emoji 在 16sp 下会被系统画成彩色方块，
                // 压过空格键本身；这个符号是单色字形，跟着 fnText 颜色走。
                key.kind == KeyKind.SPACE -> "⌾ 空格"
                else -> key.label
            }
            gravity = Gravity.CENTER
            // 数字/符号页的键比字母页宽得多，字符也跟着放大一点，不然看着像空的。
            // **调试大键盘的字号必须同比放大**：键高放大 2.6 倍而字不变，
            // 看起来像一排空盒子，反而更难确认「我点的是哪个键」。
            val sizeScale = UiPrefs.keyScale(ctx)
            setTextSize(
                TypedValue.COMPLEX_UNIT_SP,
                keyTextSizeSp(key) * sizeScale,
            )
            // 点亮的 ⇧ 用反色（底色是强调色），其余按「可打的字 / 功能键」分两档
            setTextColor(
                when {
                    lit -> p.onAccent
                    key.kind == KeyKind.CHAR || key.kind == KeyKind.TEXT -> p.keyText
                    else -> p.fnText
                },
            )
            background = keyBackground(ctx, key, shift)
            isClickable = true
            contentDescription = accessibilityLabel(key, shift)
            // 空格键的点击由下面的 OnTouch 自己触发（见那里注释），
            // 这里不能再挂 OnClickListener，否则一旦某条路径同时走了两边
            // 就会打出两个空格。
            if (key.kind != KeyKind.SPACE) {
                setOnClickListener { onKey(key) }
            }
        }
        if (key.kind == KeyKind.SHIFT) view.tag = SHIFT_TAG
        // 中/英键要能在切模式时就地改字（中 ⇄ 英），所以打上标记。
        if (key.kind == KeyKind.MODE) view.tag = MODE_TAG

        // 删除键长按连续删。手感和搜狗对齐：先删一个，然后约 50ms 一个。
        // 用 setOnLongClickListener 而不是自己算触摸时长——输入法窗口里
        // 手算时长会和宿主的触摸分发打架。
        if (key.kind == KeyKind.DELETE) {
            view.setOnLongClickListener {
                view.postDelayed(object : Runnable {
                    override fun run() {
                        if (!view.isPressed) return
                        onKey(key)
                        view.postDelayed(this, 50)
                    }
                }, 350)
                true
            }
        }

        // 空格键：**普通点击 + 长按说话**，两套手势共存。
        //
        // ## 为什么必须自己接管触摸并自己触发点击
        //
        // 试过「OnTouch 返回 false 放行给 OnClickListener」，在真机上是坏的：
        // DOWN 返回 false 后，View 不认为自己开始了一次手势，后续 UP 也就
        // 不再回传给这个 OnTouchListener——但 `isPressed` 已经在 DOWN 里被设成
        // true，于是用户看到键亮了（「页面上有反应」），可点击永远不触发
        // （「没法输入空格」）。
        //
        // 结论：接管就完整接管，UP 时自己调 onKey(key)，一次都别指望系统。
        if (key.kind == KeyKind.SPACE) {
            val slop = VOICE_CANCEL_SLOP_DP * ctx.resources.displayMetrics.density
            var downX = 0f
            var downY = 0f
            var moved = false
            view.setOnTouchListener { v, ev ->
                when (ev.actionMasked) {
                    MotionEvent.ACTION_DOWN -> {
                        v.isPressed = true
                        voiceHoldTriggered = false
                        held = true
                        moved = false
                        downX = ev.x
                        downY = ev.y
                        v.postDelayed(voiceHoldRunnable, VOICE_HOLD_MS)
                        // 消费 DOWN：既然要自己判 UP，就必须全程接管，
                        // 否则拿不到后续事件。
                        true
                    }
                    MotionEvent.ACTION_MOVE -> {
                        // 手指滑出键帽范围就取消（既有长按，也有这次点击）：
                        // 键盘上滑出去一般是想改主意，不该还把空格打上去。
                        if (!moved && (Math.abs(ev.x - downX) > slop ||
                                Math.abs(ev.y - downY) > slop)
                        ) {
                            moved = true
                            held = false
                            v.isPressed = false
                            v.removeCallbacks(voiceHoldRunnable)
                        }
                        true
                    }
                    MotionEvent.ACTION_UP -> {
                        v.isPressed = false
                        v.removeCallbacks(voiceHoldRunnable)
                        held = false
                        if (voiceHoldTriggered) {
                            // 长按走了语音：松手 = 说完了，不再补一个空格
                            voiceHoldTriggered = false
                            onVoiceUp()
                        } else if (!moved) {
                            // 普通点击：自己上屏空格
                            onKey(key)
                        }
                        true
                    }
                    MotionEvent.ACTION_CANCEL -> {
                        v.isPressed = false
                        v.removeCallbacks(voiceHoldRunnable)
                        held = false
                        // 被系统取消（来电、窗口失焦）：如果在录音就收尾，
                        // 但不打空格——用户的手指动作并不是「点了一下」。
                        if (voiceHoldTriggered) {
                            voiceHoldTriggered = false
                            onVoiceUp()
                        }
                        true
                    }
                    else -> false
                }
            }
        }

        // 上档双击 = 锁定/解锁（单击仍是一次性）。判时间戳的状态在 FeiheIme 里，
        // 这里只把「这一次点击的时间」传出去，保持 KeyboardView 无状态。
        if (key.kind == KeyKind.SHIFT) {
            view.setOnClickListener { onKey(key) }
            return view
        }
        // 有上滑字符的键要包一层：键帽右上角画小字，并接管触摸判方向。
        // 长按类（⌫/⇧）没有上滑，两套手势不会撞。
        if (key.swipe != null) {
            return withSwipe(ctx, view, key, onKey)
        }
        return view
    }

    /**
     * 给键加「右上角小字 + 上滑输入」。
     *
     * 为什么用 FrameLayout 包一层而不是在 TextView 里画：小字要贴在右上角，而键的
     * 主字符必须居中——一个 TextView 装不下两个对齐方式。
     *
     * **触摸为什么要自己接管**：setOnClickListener 拿不到「手指往哪滑」。这里 DOWN
     * 时记下 y，MOVE 里判断位移，UP 时决定是「上滑」还是「点击」；因为返回 true 会
     * 吃掉事件，点击得自己调 performClick()。
     */
    private fun withSwipe(
        ctx: Context,
        keyView: TextView,
        key: Key,
        onKey: (Key) -> Unit,
    ): View {
        val p = Palette.of(ctx)
        val hint = TextView(ctx).apply {
            text = key.swipe
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 10f)
            // 用「次要文字」而不是「最淡的提示」：键帽是白底，最淡那一档几乎看不见
            setTextColor(p.sub)
            setPadding(dp(ctx, 4f), dp(ctx, 1f), dp(ctx, 4f), 0)
            isClickable = false
        }
        val holder = FrameLayout(ctx).apply {
            isClickable = true
            addView(
                keyView,
                FrameLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    ViewGroup.LayoutParams.MATCH_PARENT,
                ),
            )
            addView(
                hint,
                FrameLayout.LayoutParams(
                    ViewGroup.LayoutParams.WRAP_CONTENT,
                    ViewGroup.LayoutParams.WRAP_CONTENT,
                    Gravity.TOP or Gravity.END,
                ),
            )
            contentDescription = "按键 ${key.label}，上滑输入 ${key.swipe}"
        }
        val baseText = keyView.text
        val basePx = keyView.textSize
        val baseColor = keyView.currentTextColor
        var previewing = false
        // 上滑到位就把键帽换成要上屏的字符：键盘上没有「悬浮气泡」那种额外窗口
        // （输入法窗口里加 PopupWindow 很容易抢焦点/被系统盖住），改成原地预览，
        // 效果一样看得见。
        fun preview(on: Boolean) {
            if (on == previewing) return
            previewing = on
            keyView.text = if (on) key.swipe ?: baseText else baseText
            keyView.setTextSize(
                TypedValue.COMPLEX_UNIT_PX,
                if (on) basePx * 1.1f else basePx,
            )
            keyView.setTextColor(if (on) p.accent else baseColor)
        }
        val threshold = dp(ctx, 14f).toFloat()
        var downY = 0f
        var swiped = false
        // **监听必须挂在键本身**：事件先派给可点的子 View，挂在外层 FrameLayout 上
        // 永远收不到 DOWN（被 keyView 吃掉了），表现就是「上滑完全没反应」。
        keyView.setOnTouchListener { v, e ->
            when (e.actionMasked) {
                MotionEvent.ACTION_DOWN -> {
                    downY = e.rawY
                    swiped = false
                    keyView.isPressed = true
                    true
                }
                MotionEvent.ACTION_MOVE -> {
                    // 只认「明显向上」：横向抖动和轻微下滑都不算，否则点一下字母
                    // 手抖几个像素就变成打数字了
                    if (!swiped && downY - e.rawY > threshold) {
                        swiped = true
                        keyView.isPressed = false
                        preview(true)
                    }
                    true
                }
                MotionEvent.ACTION_UP -> {
                    keyView.isPressed = false
                    preview(false)
                    if (swiped) {
                        // 上滑字符的 kind **跟随原键**，不能一律当 TEXT。
                        //
                        // TEXT 是「直接上屏、不碰候选」，而 PUNCT 是「先把候选上屏
                        // 再粘标点」。底排的 `，` 键上滑出 `。` 时如果按 TEXT 处理，
                        // 用户打着拼音上滑句号，会得到「lmhdilvi。」这种拼音标点混串
                        // ——PUNCT 分支里的 commitPending 那一步被跳过了。
                        //
                        // 字母键的上滑仍然走 TEXT：那些是 `1` `！` `，` 这类独立符号，
                        // 语义上确实不该触发候选上屏（打 `q` 上滑出 `1` 时把半截拼音
                        // 顶上去反而更怪）。只有 PUNCT 键需要保序语义。
                        val kind = if (key.kind == KeyKind.PUNCT) KeyKind.PUNCT else KeyKind.TEXT
                        key.swipe?.let { onKey(Key(it, kind, it)) }
                    } else {
                        v.performClick()
                    }
                    true
                }
                MotionEvent.ACTION_CANCEL -> {
                    keyView.isPressed = false
                    preview(false)
                    swiped = false
                    false
                }
                else -> false
            }
        }
        return holder
    }

    private fun keyBackground(
        ctx: Context,
        key: Key,
        shift: ShiftState = ShiftState.OFF,
    ): android.graphics.drawable.Drawable {
        val p = Palette.of(ctx)
        val radius = dp(ctx, 6f).toFloat()

        fun base(): GradientDrawable = GradientDrawable().apply {
            cornerRadius = radius
            setColor(
                when {
                    // 上档键点亮：和候选第一个同色的强调蓝，一眼看出「下一下是大写」
                    key.kind == KeyKind.SHIFT && shift != ShiftState.OFF -> p.accent
                    // 数字选词行：淡强调色底，和"能打出字"的键帽区分开——它选的是候选，
                    // 不是往组合串里加字符。
                    key.kind == KeyKind.SELECT -> p.selectBg
                    // 数字/标点页上的键和字母一样是「可打的字」，给键帽色；
                    // 功能键才用另一档灰，靠颜色就能分出「这排是打字的还是操作的」。
                    key.kind == KeyKind.CHAR || key.kind == KeyKind.TEXT -> p.keyBg
                    key.kind == KeyKind.SPACE -> p.spaceBg
                    else -> p.fnBg
                }
            )
            // 底部一条深色描边当"键帽影子"，纯色方块看着不像能按的东西
            setStroke(dp(ctx, 1f), p.stroke)
        }

        // 按下态：整键压暗一档。以前只有一个静态 GradientDrawable，手指按下去
        // 屏幕毫无变化，用起来像「没点着」，尤其在打长句时很没底。
        // 用 StateListDrawable 而不是在 OntouchListener 里改背景，是因为键的
        // 背景还有 ⇧ 点亮等状态，手势回调里改会把那些状态冲掉。
        val pressed = GradientDrawable().apply {
            cornerRadius = radius
            setColor(blend(p.keyBg, 0xFF000000.toInt(), 0.18f))
            setStroke(dp(ctx, 1f), p.stroke)
        }
        return android.graphics.drawable.StateListDrawable().apply {
            addState(intArrayOf(android.R.attr.state_pressed), pressed)
            addState(intArrayOf(), base())
        }
    }

    /**
     * 把 [color] 向 [toward] 混 [ratio]，用来生成按下态颜色。
     *
     * 手写而不是用 `ColorUtils`：那要引入 androidx.core，为一个小函数把整个
     * core 拖进来不值。ARGB 四个通道分别插值即可。
     */
    private fun blend(color: Int, toward: Int, ratio: Float): Int {
        val r = ratio.coerceIn(0f, 1f)
        fun ch(shift: Int): Int {
            val a = (color shr shift) and 0xFF
            val b = (toward shr shift) and 0xFF
            return (a + (b - a) * r).toInt().coerceIn(0, 255) shl shift
        }
        return ch(24) or ch(16) or ch(8) or ch(0)
    }

    private fun accessibilityLabel(key: Key, shift: ShiftState = ShiftState.OFF): String = when (key.kind) {
        KeyKind.CHAR, KeyKind.TEXT -> key.label.ifEmpty { "空白键" }
        KeyKind.DELETE -> "删除"
        KeyKind.SHIFT -> shiftAccessibilityLabel(shift)
        KeyKind.SPACE -> "空格"
        KeyKind.ENTER -> "换行"
        KeyKind.PUNCT -> key.label
        KeyKind.MODE -> "切换中英文"
        KeyKind.SELECT -> "选第 ${key.code} 个候选"
        // 按钮上写的是「123」/「ABC」/「符」/「😊」，读屏念这些没意义，念出「切到哪」才行
        KeyKind.PAGE -> accessibilityPageLabel(key.code)
        KeyKind.NAV -> navAccessibilityLabel(key.code)
    }

    /**
     * 导航键的读屏文案。
     *
     * 方向键的 label 本来就是「←」，读屏念出来是「左箭头」而不是「左移光标」——
     * 对看不见屏幕的人，「左箭头」这四个字不能告诉他按下去会发生什么。
     */
    private fun navAccessibilityLabel(code: String): String =
        NavAction.of(code)?.let {
            when (it) {
                NavAction.LEFT -> "光标左移"
                NavAction.RIGHT -> "光标右移"
                NavAction.UP -> "光标上移"
                NavAction.DOWN -> "光标下移"
                NavAction.SELECT_ALL -> "全选"
                NavAction.COPY -> "复制"
                NavAction.PASTE -> "粘贴"
                NavAction.CUT -> "剪切"
                NavAction.LINE_START -> "移到行首"
                NavAction.LINE_END -> "移到行尾"
            }
        } ?: "控制键"

    /**
     * 换页键的读屏文案。
     *
     * 放在这里而不是写成一串 `when(code)`：新增页面时**只加一个分支**，
     * 忘了加的后果是读屏念出英文 code——比念错更糟，但也比直接崩好查。
     */
    private fun accessibilityPageLabel(code: String): String = when (code) {
        "letters" -> "字母键盘"
        "digits" -> "数字键盘"
        "symbols" -> "符号键盘"
        "emoji" -> "表情键盘"
        else -> "切换键盘"
    }

    private fun dp(ctx: Context, v: Float): Int =
        (v * ctx.resources.displayMetrics.density + 0.5f).toInt()
}

/**
 * 键帽文字的字号（sp，未乘缩放）。
 *
 * ## 只有回车单独一档
 *
 * 以前是两档：字母/文本键 20，其余一律 16。问题是回车键宽 1.8 倍、是底排最宽的键，
 * 而 `↵` 这个字形本身笔画细、留白多，16sp 摊在那么大一块键帽上**又细又小**，
 * 用户原话「回车键上的符号也太单薄或者太小」。
 *
 * 所以只给回车抬到 22sp，**其余功能键保持 16sp 不动**——用户明确说过
 * 「其他按键体积不大」，不要顺手把 ⇧ ⌫ 123 中 ⌘ 都放大，那会让底排拥挤。
 *
 * 22 而不是更大：回车右侧紧邻键盘边界，`↵` 放太大会贴边；22sp 已经比
 * 同字号汉字显得实一档（符号笔画比汉字细），视觉上刚好压过键面积差。
 */
internal fun keyTextSizeSp(key: Key): Float = when (key.kind) {
    KeyKind.CHAR, KeyKind.TEXT -> 20f
    KeyKind.ENTER -> 26f
    else -> 16f
}

/**
 * 形码态键帽两行文字相对 [keyTextSizeSp] 基值的缩放。
 *
 * 用户原话「可以用小一些的字号」：旧值形码 0.82 / 字母 0.55，两个汉字摊在
 * 约 36dp 宽的键上发胀。整体压一档到 0.60 / 0.45（20sp 基值 → 12sp / 9sp），
 * 仍清晰但不撑键帽。测试 `AssistKeyFaceTest` 钉死这两个值和「提示必须比字母大」。
 */
internal const val ASSIST_HINT_SCALE = 0.60f
internal const val ASSIST_LETTER_SCALE = 0.45f
