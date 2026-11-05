package com.feihe.ime.keyboard

import android.content.Context
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.LinearLayout
import android.widget.TextView

/**
 * 把 [QwertyLayout.rows] 画成视图。
 *
 * 刻意不用 XML 布局：键位的**真值**是 `KeyboardLayout.kt` 里那份数据，XML 只是它的
 * 一个渲染结果。分成两份的话，改键位要改两处，而且 XML 那份没法单测。
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
        shiftActive: Boolean = false,
        onKey: (Key) -> Unit,
    ): View {
        val root = LinearLayout(ctx).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(0xFFD9DCE1.toInt())
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
                dp(ctx, KEY_HEIGHT_DP),
            ).apply { bottomMargin = dp(ctx, GAP_DP) }

            for (key in row) {
                rowView.addView(makeKey(ctx, key, shiftActive, onKey), keyParams(ctx, key))
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
     * 上档状态变化时调用：只改 ⇧ 那一颗键的底色和字色。
     *
     * 字母键的标签本来就是大写的（手机键盘的通行做法），所以**状态只能靠 ⇧ 自己
     * 表示**——不染色的用户根本分不清刚才那一下有没有点上。
     */
    fun applyShiftState(root: View?, active: Boolean) {
        val v = root?.findViewWithTag<TextView>(SHIFT_TAG) ?: return
        v.background = keyBackground(root.context, Key("⇧", KeyKind.SHIFT), active)
        v.setTextColor(if (active) Color.WHITE else 0xFF333333.toInt())
    }

    private fun makeKey(ctx: Context, key: Key, shiftActive: Boolean, onKey: (Key) -> Unit): View {
        val lit = key.kind == KeyKind.SHIFT && shiftActive
        val view = TextView(ctx).apply {
            text = key.label
            gravity = Gravity.CENTER
            setTextSize(TypedValue.COMPLEX_UNIT_SP, if (key.kind == KeyKind.CHAR) 20f else 16f)
            // 点亮的 ⇧ 用白字（底色是强调蓝），其余按老规矩黑/深灰
            setTextColor(
                when {
                    lit -> Color.WHITE
                    key.kind == KeyKind.CHAR -> Color.BLACK
                    else -> 0xFF333333.toInt()
                },
            )
            background = keyBackground(ctx, key, shiftActive)
            isClickable = true
            contentDescription = accessibilityLabel(key)
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
        return view
    }

    private fun keyBackground(ctx: Context, key: Key, shiftActive: Boolean = false): GradientDrawable {
        val radius = dp(ctx, 6f).toFloat()
        return GradientDrawable().apply {
            cornerRadius = radius
            setColor(
                when {
                    // 上档键点亮：和候选第一个同色的强调蓝，一眼看出「下一下是大写」
                    key.kind == KeyKind.SHIFT && shiftActive -> 0xFF1A6FD4.toInt()
                    // 数字/标点页上的键和字母一样是「可打的字」，给白底；
                    // 功能键才用灰底，靠颜色就能分出「这排是打字的还是操作的」。
                    key.kind == KeyKind.CHAR || key.kind == KeyKind.TEXT -> Color.WHITE
                    key.kind == KeyKind.SPACE -> 0xFFF7F7F8.toInt()
                    else -> 0xFFB9BEC6.toInt()
                }
            )
            // 底部一条深色描边当"键帽影子"，纯色方块看着不像能按的东西
            setStroke(dp(ctx, 1f), 0xFF9AA0A8.toInt())
        }
    }

    private fun accessibilityLabel(key: Key): String = when (key.kind) {
        KeyKind.CHAR, KeyKind.TEXT -> key.label
        KeyKind.DELETE -> "删除"
        KeyKind.SHIFT -> "上档（下一个字母大写）"
        KeyKind.SPACE -> "空格"
        KeyKind.ENTER -> "换行"
        KeyKind.PUNCT -> key.label
        KeyKind.MODE -> "切换中英文"
        // 按钮上写的是「123」/「ABC」，读屏念数字没有意义，念出「切到哪」才行
        KeyKind.SYMBOL -> if (key.label == "ABC") "字母键盘" else "数字和符号"
    }

    private fun dp(ctx: Context, v: Float): Int =
        (v * ctx.resources.displayMetrics.density + 0.5f).toInt()
}
