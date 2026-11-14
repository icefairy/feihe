package com.feihe.ime.voice

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 语音纠错的规格锁。
 *
 * 重点不是「能不能纠」，而是**什么时候绝对不能纠**——这个功能会改写用户说的话，
 * 改错一次比一百次没纠都糟。
 */
class VoiceCorrectTest {

    /** 一张假拼音表：只为单测服务，不碰真实词库。 */
    private val pinyinMap = mapOf(
        "退" to "tui", "格" to "ge", "革" to "ge",
        "飞" to "fei", "鹤" to "he", "和" to "he", "菲" to "fei",
        "键" to "jian", "盘" to "pan",
        "张" to "zhang", "三" to "san",
    )
    private val pinyinOf = VoiceCorrect.PinyinOf { pinyinMap[it] ?: "" }

    private fun phrase(word: String, pinyin: String) = VoiceCorrect.Phrase(word, pinyin)
    /** 带使用频率的候选（用户 2026-11-14：频率高的优先）。 */
    private fun phrase(word: String, pinyin: String, freq: Long) =
        VoiceCorrect.Phrase(word, pinyin, freq)

    // ---- 同音同长：按用户频率优先 ----

    @Test
    fun `同音同长时频率高的候选胜出`() {
        // 「菲和」ASR 原字，词库里「飞鹤」「飞和」都念 fei/he、都是 2 字，
        // 都能替换。用户天天打「飞鹤」(freq 9)，就该选它而不是少用的「飞和」。
        val r = VoiceCorrect.correct(
            "菲和",
            listOf(
                phrase("飞和", "fei he", 1),   // 故意放前面：文件序靠前也不能赢
                phrase("飞鹤", "fei he", 9),   // 高频
            ),
            pinyinOf,
        )
        assertEquals("飞鹤", r.text)
    }

    @Test
    fun `频率反过来时低频候选不抢高频`() {
        val r = VoiceCorrect.correct(
            "菲和",
            listOf(
                phrase("飞鹤", "fei he", 1),
                phrase("飞和", "fei he", 9),
            ),
            pinyinOf,
        )
        assertEquals("飞和", r.text)
    }

    @Test
    fun `不带频率时退化为稳定排序_行为与旧版一致`() {
        // 两个候选 freq 都默认 0，Kotlin 稳定排序保持列表原序
        val r = VoiceCorrect.correct(
            "菲和",
            listOf(phrase("飞和", "fei he"), phrase("飞鹤", "fei he")),
            pinyinOf,
        )
        assertEquals("飞和", r.text)
    }

    // ---- 该纠的 ----

    @Test
    fun `拼音完全吻合时替换成自造词`() {
        val r = VoiceCorrect.correct("退革", listOf(phrase("退格", "tui ge")), pinyinOf)
        assertEquals("退格", r.text)
        assertTrue(r.changed)
        assertEquals(listOf("退革" to "退格"), r.replaced)
    }

    @Test
    fun `句中片段也能替换`() {
        val r = VoiceCorrect.correct(
            "我按退革试试",
            listOf(phrase("退格", "tui ge")),
            pinyinOf,
        )
        assertEquals("我按退格试试", r.text)
    }

    @Test
    fun `长词优先，不被短词抢前缀`() {
        // 词库里同时有「退格」和「退格键」，必须整体匹配长的
        val r = VoiceCorrect.correct(
            "点退革键",
            listOf(phrase("退格", "tui ge"), phrase("退格键", "tui ge jian")),
            pinyinOf,
        )
        assertEquals("点退格键", r.text)
    }

    // ---- 绝对不能纠的 ----

    @Test
    fun `拼音差一个音节就不动`() {
        // 「退格」tui ge vs 「退格」的误识「退回」tui hui —— hui 不在表里
        val r = VoiceCorrect.correct("退回", listOf(phrase("退格", "tui ge")), pinyinOf)
        assertEquals("退回", r.text)
        assertFalse(r.changed)
    }

    @Test
    fun `长度不一致不替换`() {
        // 「退」单独一个字，词库词是两字——长度对不上
        val r = VoiceCorrect.correct("退", listOf(phrase("退格", "tui ge")), pinyinOf)
        assertEquals("退", r.text)
        assertFalse(r.changed)
    }

    @Test
    fun `同音字不会互换成词库外的结果`() {
        // 「飞和」的拼音是 fei he，「飞鹤」也是 fei he —— 这种情况该纠
        val r = VoiceCorrect.correct("飞和", listOf(phrase("飞鹤", "fei he")), pinyinOf)
        assertEquals("飞鹤", r.text)
    }

    @Test
    fun `查不到拼音的字不参与纠错`() {
        // 「某某」两个字的拼音都查不到，词库里有「某某」也不该动
        val r = VoiceCorrect.correct("某某", listOf(phrase("某某", "mou mou")), pinyinOf)
        assertEquals("某某", r.text)
        assertFalse(r.changed)
    }

    @Test
    fun `空词库原样返回`() {
        val r = VoiceCorrect.correct("退革", emptyList(), pinyinOf)
        assertEquals("退革", r.text)
        assertFalse(r.changed)
    }

    @Test
    fun `原文已经正确时不产生替换记录`() {
        val r = VoiceCorrect.correct("退格", listOf(phrase("退格", "tui ge")), pinyinOf)
        assertEquals("退格", r.text)
        assertFalse(r.changed)
        assertTrue(r.replaced.isEmpty())
    }

    // ---- 边界 ----

    @Test
    fun `标点和字母原样保留`() {
        val r = VoiceCorrect.correct(
            "按退革，然后 Abc",
            listOf(phrase("退格", "tui ge")),
            pinyinOf,
        )
        assertEquals("按退格，然后 Abc", r.text)
    }

    @Test
    fun `多处出现都替换`() {
        val r = VoiceCorrect.correct(
            "退革一次再退革一次",
            listOf(phrase("退格", "tui ge")),
            pinyinOf,
        )
        assertEquals("退格一次再退格一次", r.text)
        assertEquals(2, r.replaced.size)
    }

    @Test
    fun `拼音音节数不匹配词长时放弃`() {
        // 词是两个字但只给了一个音节：数据本身坏了，不能用
        val r = VoiceCorrect.correct("退革", listOf(phrase("退格", "tui")), pinyinOf)
        assertEquals("退革", r.text)
    }
}
