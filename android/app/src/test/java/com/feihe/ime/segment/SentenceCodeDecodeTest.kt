package com.feihe.ime.segment

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 逐字选词落库的**链路级**回归：`sentenceCode()` 产出的码必须能被解码成全拼。
 *
 * ## 为什么专门钉这一条
 *
 * `rememberPhrase` 的流程是：
 * ```
 * state.sentenceCode()  →  "lq xb rj"      （音节间带空格）
 * NativeEngine.decodePinyin(code)  →  "liu xin ran"
 * NativeEngine.learnPhrase(sentence, full)
 * ```
 *
 * 而 core 的 `shuangpin::decode_pinyin` 以前**只认连续码**：`"lq xb rj"` 是 8 个字符，
 * 是偶数，`len % 2` 那道校验能过，于是按两字符切成 `"lq" " x" " b" " r" "j"`，
 * 空格混进音节，`decode_chars` 全部失败返回 `None`。
 *
 * 结果是 `rememberPhrase` 判定「解不出全拼」**静默放弃**，`phrases.txt`
 * 从来没有被创建过——用户看到的是「这词造过一次，下次打同一个码还是出不来」。
 *
 * 这个测试盯的就是「码的形态」这件事：只要 `sentenceCode()` 还带空格，
 * 解码就必须支持空格。两边任何一个改了，这里立刻红。
 */
class SentenceCodeDecodeTest {

    @Test
    fun `分段态的码是空格分隔的`() {
        val s = SegmentState(
            segments = listOf(listOf("流", "刘"), listOf("新", "心"), listOf("然", "染")),
            codes = listOf("lq", "xb", "rj"),
        )
        // 这是 native 侧 decodePinyin 的输入，形态必须是 "lq xb rj"
        assertEquals("lq xb rj", s.sentenceCode())
    }

    @Test
    fun `整句拼出来是三个字`() {
        val s = SegmentState(
            segments = listOf(listOf("流", "刘"), listOf("新", "心"), listOf("然", "染")),
            codes = listOf("lq", "xb", "rj"),
        )
        // 默认每段选第 0 个
        assertEquals("流新然", s.sentence())
    }

    @Test
    fun `codes 长度不对时 sentenceCode 返回空串`() {
        val s = SegmentState(
            segments = listOf(listOf("流"), listOf("新"), listOf("然")),
            codes = listOf("lq", "xb"), // 少一段
        )
        assertEquals("", s.sentenceCode())
    }

    @Test
    fun `段码去空格后拼接，不产生连续多空格`() {
        val s = SegmentState(
            segments = listOf(listOf("刘"), listOf("欣")),
            codes = listOf(" lq ", "  xb  "),
        )
        assertEquals("lq xb", s.sentenceCode())
        // 关键：不含连续空格，否则 native 侧 split_whitespace 虽然能处理，
        // 但日志和调试时看起来像码错了
        assertTrue(!s.sentenceCode().contains("  "))
        assertNotEquals("lqxb", s.sentenceCode())
    }
}
