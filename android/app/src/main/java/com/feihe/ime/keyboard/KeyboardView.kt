package com.feihe.ime.keyboard

import android.content.Context
import android.graphics.drawable.GradientDrawable
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.LinearLayout
import android.widget.TextView
import com.feihe.ime.Palette

/**
 * 把 [KeyboardPages] 的行数据画成一棵 View 树。
 *
 * **颜色一律取自 [Palette]**，这个类里不允许再出现字面量色值——深色模式下
 * 漏一处就是一块刺眼的白底（或黑底黑字）。深浅两套由 Palette 按系统夜间模式选。
 */
object KeyboardView {

    private const val KEY_HEIGHT_DP = 46f
    private const val GAP_DP = 5f

    /**
     * 上档键的 tag。
     *
     * 上档状态变了**不重建整个键盘**：重建会闪一下、候选条也跟着抖，而且用户只是
     * 按了一下 ⇧。改成按 tag 找到那颗键、就地换配色。
     */
    const val SHIFT_TAG = "feihe.shift"

    fun build(
        ctx: Context,
        rows: List<List<Key>>,
        shift: ShiftState = ShiftState.OFF,
        rowHeightDp: Float = KEY_HEIGHT_DP,
        onLongKey: (Key) -> Unit = {},
        onKey: (Key) -> Unit,
    ): View {
        val p = Palette.of(ctx)
        val root = LinearLayout(ctx).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(p.keyboardBg)
            setPadding(dp(ctx, 3f), dp(ctx, 6f), dp(ctx, 3f), dp(ctx, 6f))
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
                rowView.addView(makeKey(ctx, key, shift, onLongKey, onKey), keyParams(ctx, key))
            }
            root.addView(rowView, lp)
        }
        return root
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
        onKey: (Key) -> Unit,
    ): View {
        val p = Palette.of(ctx)
        val lit = key.kind == KeyKind.SHIFT && shift != ShiftState.OFF
        val view = TextView(ctx).apply {
            text = if (key.kind == KeyKind.SHIFT) shiftLabel(shift) else key.label
            gravity = Gravity.CENTER
            // 数字/符号页的键比字母页宽得多，字符也跟着放大一点，不然看着像空的
            setTextSize(
                TypedValue.COMPLEX_UNIT_SP,
                if (key.kind == KeyKind.CHAR || key.kind == KeyKind.TEXT) 20f else 16f,
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
            setOnClickListener { onKey(key) }
        }
        if (key.kind == KeyKind.SHIFT) view.tag = SHIFT_TAG

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

        // 上档长按 = 锁定/解锁。**不用双击**：和「连按两下打两个大写」抢时间窗口，
        // 阈值松了误触、紧了按不出来；长按是这套键盘已有的手势（⌫ 长按连续删）。
        if (key.kind == KeyKind.SHIFT) {
            view.setOnLongClickListener {
                onLongKey(key)
                true
            }
        }
        return view
    }

    private fun keyBackground(
        ctx: Context,
        key: Key,
        shift: ShiftState = ShiftState.OFF,
    ): GradientDrawable {
        val p = Palette.of(ctx)
        val radius = dp(ctx, 6f).toFloat()
        return GradientDrawable().apply {
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
    }

    private fun accessibilityLabel(key: Key, shift: ShiftState = ShiftState.OFF): String = when (key.kind) {
        KeyKind.CHAR, KeyKind.TEXT -> key.label
        KeyKind.DELETE -> "删除"
        KeyKind.SHIFT -> shiftAccessibilityLabel(shift)
        KeyKind.SPACE -> "空格"
        KeyKind.ENTER -> "换行"
        KeyKind.PUNCT -> key.label
        KeyKind.MODE -> "切换中英文"
        KeyKind.SELECT -> "选第 ${key.code} 个候选"
        // 按钮上写的是「123」/「ABC」/「符」/「😊」，读屏念这些没意义，念出「切到哪」才行
        KeyKind.PAGE -> accessibilityPageLabel(key.code)
    }

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
