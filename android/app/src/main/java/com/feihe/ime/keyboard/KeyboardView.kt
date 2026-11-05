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

    fun build(ctx: Context, rows: List<List<Key>>, onKey: (Key) -> Unit): View {
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
                rowView.addView(makeKey(ctx, key, onKey), keyParams(ctx, key))
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

    private fun makeKey(ctx: Context, key: Key, onKey: (Key) -> Unit): View {
        val view = TextView(ctx).apply {
            text = key.label
            gravity = Gravity.CENTER
            setTextSize(TypedValue.COMPLEX_UNIT_SP, if (key.kind == KeyKind.CHAR) 20f else 16f)
            setTextColor(if (key.kind == KeyKind.CHAR) Color.BLACK else 0xFF333333.toInt())
            background = keyBackground(ctx, key)
            isClickable = true
            contentDescription = accessibilityLabel(key)
            setOnClickListener { onKey(key) }
        }

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

    private fun keyBackground(ctx: Context, key: Key): GradientDrawable {
        val radius = dp(ctx, 6f).toFloat()
        return GradientDrawable().apply {
            cornerRadius = radius
            setColor(
                when (key.kind) {
                    KeyKind.CHAR -> Color.WHITE
                    KeyKind.SPACE -> 0xFFF7F7F8.toInt()
                    else -> 0xFFB9BEC6.toInt()
                }
            )
            // 底部一条深色描边当"键帽影子"，纯色方块看着不像能按的东西
            setStroke(dp(ctx, 1f), 0xFF9AA0A8.toInt())
        }
    }

    private fun accessibilityLabel(key: Key): String = when (key.kind) {
        KeyKind.CHAR -> key.label
        KeyKind.DELETE -> "删除"
        KeyKind.SHIFT -> "上档"
        KeyKind.SPACE -> "空格"
        KeyKind.ENTER -> "换行"
        KeyKind.PUNCT -> key.label
        KeyKind.MODE -> "切换中英文"
        KeyKind.SYMBOL -> "符号"
    }

    private fun dp(ctx: Context, v: Float): Int =
        (v * ctx.resources.displayMetrics.density + 0.5f).toInt()
}
