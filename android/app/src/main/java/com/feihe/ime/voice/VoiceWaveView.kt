package com.feihe.ime.voice

import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.view.View
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min

/**
 * 录音时的音量波形。
 *
 * ## 为什么至少要有这个东西
 *
 * 语音输入最让人不安的是「它到底听见我了吗」。旧版只有一个🎤变成🔴，用户
 * 只能等结果——说三秒、没出字，就无法判断是麦克风没收到声音、还是模型没听懂。
 * 波形把「有在收音」这件事直接画出来：说话时条子跳，不说话时贴着底。
 *
 * ## 为什么自己画而不放一个动画
 *
 * 播放一段假的循环动画是骗人的——用户对着手机喊，波形跳得欢，结果是空的，
 * 下次就不信这个指示了。这里画的是**真实 RMS**：取自每一帧 PCM 的均方根，
 * 所以波形平就是真的没收到声音，这是可诊断的信息，不是装饰。
 *
 * ## 为什么用「右进左出」的滚动条而不是一条实时曲线
 *
 * 滚动条能让人一眼看出「刚才这三秒有没有断」——断音会留下一段贴底的空槽。
 * 单条随音量上下跳的曲线做不到（它只有当下，没有历史）。
 */
class VoiceWaveView(ctx: Context) : View(ctx) {

    /** 每根条子的宽度（px）。太细在低分辨率上会糊成一片。 */
    private val barWidth = dp(3f)
    private val barGap = dp(2f)

    /**
     * 条子的最小高度。
     *
     * 不能是 2dp 那种「几乎看不见」的值：静音时整排条子贴在底部的样子，用户分不清
     * 是「在听但你没说话」还是「这个控件压根没出来」。留 4dp 让整排有个明确的
     * 底噪轮廓——正常说话时条子高过它，一眼就能看出「有在收音」。
     */
    private val minBarHeight = dp(4f)

    private val paint = Paint(Paint.ANTI_ALIAS_FLAG)

    /**
     * 音量历史，0f..1f，最新的在末尾。
     *
     * 长度按控件宽度算，不够就补；太长了从头部丢。用它而不是 List<Float> +
     * removeAt(0)：后者每次都是 O(n) 内存搬移，一帧一次在录音时白耗。
     */
    private var levels = FloatArray(0)

    /** 条子用主色还是提示色。识别中（不再收音）要变暗，表示「现在不是在听」。 */
    private var active = true

    /** 在 0..1 之间取一个音量。录音线程每帧调一次。 */
    fun push(level: Float) {
        val n = slotCount()
        if (n <= 0) return
        if (levels.size != n) {
            // 宽度变了（转屏/重建）：保留最近的尾巴，前面补 0
            val next = FloatArray(n)
            val keep = min(levels.size, n)
            System.arraycopy(levels, levels.size - keep, next, n - keep, keep)
            levels = next
        }
        // 整体左移一格，末尾写新值。System.arraycopy 比自己写循环快得多。
        System.arraycopy(levels, 1, levels, 0, n - 1)
        levels[n - 1] = level.coerceIn(0f, 1f)
        invalidate()
    }

    /** 停止收音：把所有条子压到底，并调暗。用户一眼看出「已经不听了」。 */
    fun freeze() {
        active = false
        levels.fill(0f)
        invalidate()
    }

    /** 重新开始收音。 */
    fun resume() {
        active = true
        levels.fill(0f)
        invalidate()
    }

    fun setTint(color: Int) {
        paint.color = color
        invalidate()
    }

    private fun slotCount(): Int {
        val usable = width - paddingLeft - paddingRight
        if (usable <= 0) return 0
        val step = barWidth + barGap
        return max(1, (usable / step).toInt())
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        val n = slotCount()
        if (n <= 0) return
        if (levels.size != n) {
            val next = FloatArray(n)
            val keep = min(levels.size, n)
            System.arraycopy(levels, levels.size - keep, next, n - keep, keep)
            levels = next
        }
        val h = height - paddingTop - paddingBottom
        val midTop = paddingTop.toFloat()
        // 条子从底部向上长，底对齐看起来像「电平」，比居中更符合直觉
        val step = barWidth + barGap
        for (i in 0 until n) {
            val lv = levels[i]
            val bh = max(minBarHeight, lv * h)
            // 不活跃时统一画成很矮的一条：不是隐藏，是「在，但没在听」
            val drawH = if (active) bh else minBarHeight
            val left = paddingLeft + i * step
            val right = left + barWidth
            val bottom = (midTop + h)
            canvas.drawRect(left, bottom - drawH, right, bottom, paint)
        }
    }

    private fun dp(v: Float): Float = v * resources.displayMetrics.density
}
