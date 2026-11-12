package com.feihe.ime.voice

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.abs

/**
 * 波形音量计算。
 *
 * 这个函数是纯计算，所以能直接测——而它必须被测，因为「波形跳」是用户判断
 * 「麦克风有没有在收音」的唯一依据。算错了（比如全返回 0）不会崩，只会让
 * 用户对着手机喊半天然后以为输入法坏了。
 */
class VoiceWaveTest {

    private fun pcm(vararg v: Int): ShortArray = ShortArray(v.size) { v[it].toShort() }

    @Test
    fun `静音贴底`() {
        assertEquals(0f, VoiceWave.levelOf(pcm(0, 0, 0, 0), 4), 1e-6f)
    }

    @Test
    fun `正常说话有明显高度`() {
        // 振幅 3000 约是正常说话的 PCM 值（满量程 32767）
        val lv = VoiceWave.levelOf(pcm(3000, -3000, 3000, -3000), 4)
        // 必须明显离开底部，否则波形看起来像没收到声音
        assertTrue("正常说话音量应 > 0.1，实际 $lv", lv > 0.1f)
    }

    @Test
    fun `喊话接近满量程`() {
        val lv = VoiceWave.levelOf(pcm(30000, -30000, 30000, -30000), 4)
        assertTrue("满量程应接近 1，实际 $lv", lv > 0.8f)
    }

    @Test
    fun `音量越大结果越大`() {
        val quiet = VoiceWave.levelOf(pcm(500, -500, 500, -500), 4)
        val normal = VoiceWave.levelOf(pcm(3000, -3000, 3000, -3000), 4)
        val loud = VoiceWave.levelOf(pcm(20000, -20000, 20000, -20000), 4)
        assertTrue("静<正常 失败：$quiet / $normal", quiet < normal)
        assertTrue("正常<响 失败：$normal / $loud", normal < loud)
    }

    @Test
    fun `峰值不主导 单个爆音不该顶满`() {
        // 真实一帧是 512 点。一个满幅尖峰混在 511 个静音里，RMS ≈ 0.044，
        // 这正是「关门声 / 手指碰麦」的量级——它不该让波形看起来像在正常说话。
        val frame = ShortArray(512)
        frame[100] = 32767
        val lv = VoiceWave.levelOf(frame, 512)
        assertTrue("单点爆音应偏低，实际 $lv", lv < 0.5f)
    }

    @Test
    fun `永远落在 0 到 1`() {
        // 全满幅方波也不能超过 1
        val lv = VoiceWave.levelOf(ShortArray(64) { if (it % 2 == 0) 32767 else -32768 }, 64)
        assertTrue("应在 [0,1]，实际 $lv", lv in 0f..1f)
    }

    @Test
    fun `空帧返回 0`() {
        assertEquals(0f, VoiceWave.levelOf(pcm(1000), 0), 1e-6f)
    }

    @Test
    fun `count 超出数组长度不越界`() {
        // 防御：调用方给的 count 大于实际数组时必须夹住，不能 ArrayIndexOutOfBounds
        val lv = VoiceWave.levelOf(pcm(1000, -1000), 999)
        assertTrue(lv >= 0f)
    }

    @Test
    fun `负值也能算出正音量`() {
        val a = VoiceWave.levelOf(pcm(2000, 2000, 2000, 2000), 4)
        val b = VoiceWave.levelOf(pcm(-2000, -2000, -2000, -2000), 4)
        assertTrue("正负振幅应得到相近音量", abs(a - b) < 1e-6f)
    }
}
