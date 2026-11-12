package com.feihe.ime.voice

import kotlin.math.min
import kotlin.math.pow
import kotlin.math.sqrt

/**
 * PCM 音量计算。
 *
 * 抽成独立对象而不是塞在 View 里，是因为它要能被 JVM 单测直接调——
 * 「说话时波形该跳、静音时该贴底」这种判据必须可验证，不能靠肉眼看。
 */
object VoiceWave {

    /**
     * 一帧 PCM16 → 0f..1f 的音量。
     *
     * ## 为什么用 RMS 而不是峰值
     *
     * 峰值受单个爆音支配：关门声、手指碰麦、手机碰到桌子，都能瞬间顶到 1.0，
     * 而正常说话时峰值也在 0.5 上下——两者在视觉上分不开，波形就失去诊断意义了。
     * RMS 反映这一段的能量，更接近人耳对「响度」的感受。
     *
     * ## 为什么要做幂次压缩
     *
     * 正常说话的 RMS 只有 0.03~0.1，线性画出来几乎全贴着底。做 rms^(2/3) 压缩后：
     *
     * | 情况 | RMS | 输出 |
     * |---|---|---|
     * | 安静 | 0.015 | 0.12 |
     * | 正常说话 | 0.09 | 0.41 |
     * | 单点爆音（512 点里一个满幅） | 0.044 | 0.25 |
     * | 喊话 | 0.92 | 1.00 |
     *
     * 关键是**爆音（0.25）低于正常说话（0.41）**——用 sqrt 压缩时爆音会
     * 反而顶到 0.55 以上，看着像在正常讲话，那就骗人了。
     */
    fun levelOf(samples: ShortArray, count: Int): Float {
        if (count <= 0) return 0f
        val n = min(count, samples.size)
        if (n <= 0) return 0f
        var sum = 0.0
        for (i in 0 until n) {
            val v = samples[i] / 32768.0
            sum += v * v
        }
        if (sum <= 0.0) return 0f
        val rms = sqrt(sum / n)
        // rms^(2/3) 再乘 2.0
        return min(1f, (rms * rms).pow(1.0 / 3.0).toFloat() * 2f)
    }
}
