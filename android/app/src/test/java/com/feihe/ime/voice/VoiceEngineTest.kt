package com.feihe.ime.voice

import org.junit.Assert.assertEquals
import org.junit.Test

/** PCM16 → 浮点样本的换算锁：归一化常数和边界值。 */
class VoiceEngineTest {

    @Test
    fun `归一化到 -1 到 1`() {
        val f = VoiceEngine.pcm16ToFloat(shortArrayOf(0, 16384, -16384, 32767, -32768))
        assertEquals(0f, f[0], 1e-6f)
        assertEquals(0.5f, f[1], 1e-6f)
        assertEquals(-0.5f, f[2], 1e-6f)
        assertEquals(32767 / 32768.0f, f[3], 1e-6f)
        assertEquals(-1.0f, f[4], 1e-6f)
    }

    @Test
    fun `长度不变`() {
        assertEquals(0, VoiceEngine.pcm16ToFloat(ShortArray(0)).size)
        assertEquals(VoiceEngine.VAD_WINDOW, VoiceEngine.pcm16ToFloat(ShortArray(VoiceEngine.VAD_WINDOW)).size)
    }

    @Test
    fun `VAD 窗口和采样率是钉死的`() {
        assertEquals(512, VoiceEngine.VAD_WINDOW)
        assertEquals(16_000, VoiceEngine.SAMPLE_RATE)
    }
}
