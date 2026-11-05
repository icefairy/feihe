package com.feihe.ime.voice

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * 句末标点的行为锁。这些断言就是规格——改 [VoicePunctuator] 之前先看它们。
 */
class VoicePunctuatorTest {

    @Test
    fun `空串保持空串`() {
        assertEquals("", VoicePunctuator.punctuate(""))
    }

    @Test
    fun `纯空白保持空串`() {
        assertEquals("", VoicePunctuator.punctuate("   \n "))
    }

    @Test
    fun `普通句子补句号`() {
        assertEquals("今天天气不错。", VoicePunctuator.punctuate("今天天气不错"))
    }

    @Test
    fun `幂等，重复调用不会变成两个句号`() {
        val once = VoicePunctuator.punctuate("今天天气不错")
        assertEquals(once, VoicePunctuator.punctuate(once))
        assertEquals("今天天气不错。", once)
    }

    @Test
    fun `已有的各种标点都不动`() {
        for (t in listOf("你好。", "走不走？", "太好了！", "等等……", "他说：“你好”", "（备注）", "五点半;")) {
            assertEquals("不该改动：$t", t, VoicePunctuator.punctuate(t))
        }
    }

    @Test
    fun `句中有逗号，只在末尾补`() {
        assertEquals("你好，今天天气不错。", VoicePunctuator.punctuate("你好，今天天气不错"))
    }

    @Test
    fun `吗结尾判问号`() {
        assertEquals("你去吗？", VoicePunctuator.punctuate("你去吗"))
    }

    @Test
    fun `呢结尾仍是句号，不当疑问句`() {
        // 「呢」在陈述句里太常见（我在家呢），判成问号是错的。
        // 判成句号只是不够好，不是错——两害相权。
        assertEquals("我在家呢。", VoicePunctuator.punctuate("我在家呢"))
    }

    @Test
    fun `单字不补标点`() {
        assertEquals("好", VoicePunctuator.punctuate("好"))
        assertEquals("嗯", VoicePunctuator.punctuate("嗯"))
    }

    @Test
    fun `两个汉字要补`() {
        assertEquals("你好。", VoicePunctuator.punctuate("你好"))
    }

    @Test
    fun `句尾空白先吃掉再加标点`() {
        assertEquals("你好。", VoicePunctuator.punctuate("你好   "))
    }

    @Test
    fun `纯英文用英文句号`() {
        assertEquals("hello world.", VoicePunctuator.punctuate("hello world"))
        assertEquals("are you ok?", VoicePunctuator.punctuate("are you ok?"))
    }

    @Test
    fun `中英混排用中文标点`() {
        assertEquals("我今天用 main 分支。", VoicePunctuator.punctuate("我今天用 main 分支"))
    }
}
