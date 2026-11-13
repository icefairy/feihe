package com.feihe.ime.voice

/**
 * 语音识别结果 × 用户自造词库的本地纠错。
 *
 * ## 它解决什么
 *
 * ASR 对**生僻词/人名/自造词**的识别率明显低于常用词：用户常打「退格」「飞鹤」，
 * 语音却听成「退革」「菲和」。而这两个词恰好在用户的自造词库里（用户天天用，
 * 输入法已经学过）——于是可以拿词库去校正 ASR 的错字。
 *
 * ## 硬约束（用户定的，一条都不能松）
 *
 * 1. **拼音必须完全吻合才替换。** 不做模糊匹配、不做相似度、不做编辑距离。
 *    差一个声母就不动——宁可留着 ASR 的原字，也不能把对的改错。
 * 2. **长度必须一致。** 四个字换两个字会让后续拼接错位，用户在光标处根本看不出
 *    哪里被动了手脚。
 * 3. **可撤销。** 纠错是自动的、用户没点头的，必须能一键还原（语音面板上留入口）。
 * 4. **默认关闭。** 会改写用户说的话，属于「宁可没有也不能错」的功能。
 *    用户不主动打开，就不该有任何一次识别结果被替换。
 *
 * ## 为什么是「拼音完全吻合」而不是「看起来像」
 *
 * 拼音是**输入法自己的真值**：自造词库里每条都存了全拼（`退格` → `tui ge`），
 * ASR 出的错字也能反查拼音。两个全拼字符串逐音节相等，才说明「这两个词念法一样」
 * ——这是客观判据，不依赖任何主观相似度阈值。
 *
 * 反过来，模糊匹配在这种场景下**必然出错**：「飞鹤」和「飞蛾」声母韵母都接近，
 * 但对用户是两个完全不同的词，改错了比不纠更糟。
 */
object VoiceCorrect {

    /**
     * 一条自造词：词面 + 全拼（音节用空格分隔，如 `tui ge`）。
     *
     * @param pinyin 全拼。空串表示这条不可用于纠错（没有可信拼音就没法比）
     */
    data class Phrase(val word: String, val pinyin: String) {
        /** 按音节的列表形式，比对时用。 */
        val syllables: List<String>
            get() = pinyin.split(' ').filter { it.isNotEmpty() }
    }

    /**
     * 一次纠错的结果。
     *
     * `text == original` 就表示没改，调用方据此决定要不要留撤销入口。
     */
    data class Result(
        val original: String,
        val text: String,
        /** 被替换掉的原文片段，撤销时用来还原 */
        val replaced: List<Pair<String, String>> = emptyList(),
    ) {
        val changed: Boolean get() = text != original
    }

    /** 单个汉字的查拼音函数：给字，返回全拼音节（无则空串）。 */
    fun interface PinyinOf {
        fun of(char: String): String
    }

    /**
     * 对 [text] 里的每一段连续汉字做纠正。
     *
     * ## 匹配策略：最长优先
     *
     * 同一段文字可能有多个候选替换（词库里有「退格」也有「退格键」），
     * 从长到短试，长的先匹配——否则「退格键」会被「退格」抢走前两个字，
     * 剩下一个「键」孤零零留在原地，结果比不纠还乱。
     *
     * @param text  ASR 原始结果
     * @param phrases 用户自造词库（词面 + 全拼）
     * @param pinyinOf 逐字查全拼
     */
    fun correct(text: String, phrases: List<Phrase>, pinyinOf: PinyinOf): Result {
        if (text.isEmpty() || phrases.isEmpty()) return Result(text, text)
        // 只留「能查到每个字拼音」的词，其余参与不了比对
        val usable = phrases.filter { it.syllables.isNotEmpty() && it.word.isNotEmpty() }
        if (usable.isEmpty()) return Result(text, text)
        // 长词优先，避免短词抢走长词的前缀
        val byLen = usable.sortedByDescending { it.word.length }

        val out = StringBuilder(text.length)
        val replaced = mutableListOf<Pair<String, String>>()
        var i = 0
        while (i < text.length) {
            val cp = text.codePointAt(i)
            val ch = String(Character.toChars(cp))
            val width = Character.charCount(cp)
            // 非汉字不参与（标点、字母、数字原样保留）
            if (!isHan(ch)) {
                out.append(ch)
                i += width
                continue
            }
            // 从当前位置试所有词，取第一个「长度一致 + 拼音逐个吻合」的
            val hit = byLen.firstOrNull { p ->
                canReplace(text, i, p, pinyinOf)
            }
            if (hit != null) {
                val n = hit.word.length
                // 长度一致是硬约束：替换前后字数必须相同（canReplace 已保证，
                // 这里再比一次是因为位运算之外还有人改这段代码时容易漏）
                val original = text.substring(i, i + n)
                if (original != hit.word) {
                    replaced += original to hit.word
                }
                out.append(hit.word)
                i += n
            } else {
                out.append(ch)
                i += width
            }
        }
        return Result(text, out.toString(), replaced)
    }

    /**
     * [text] 从 [start] 起能不能换成 [phrase]。
     *
     * 三个条件：够长、每个字都能查到拼音、拼音逐音节相等。
     */
    private fun canReplace(
        text: String,
        start: Int,
        phrase: Phrase,
        pinyinOf: PinyinOf,
    ): Boolean {
        val n = phrase.word.length
        // 长度必须一样：这是用户定的硬约束，也是「字数不变」的物理保证
        if (start + n > text.length) return false
        val slice = text.substring(start, start + n)
        // 已经一样就不用换（省掉无意义的 replaced 记录）
        if (slice == phrase.word) return true
        val sylls = phrase.syllables
        if (sylls.size != n) return false
        // 逐字比拼音：任何一字对不上就整体否决
        for (k in 0 until n) {
            val got = pinyinOf.of(slice.substring(k, k + 1))
            if (got.isEmpty() || got != sylls[k]) return false
        }
        return true
    }

    /** 是否汉字（CJK 统一表意文字，含扩展 A 区）。 */
    private fun isHan(s: String): Boolean {
        if (s.isEmpty()) return false
        val cp = s.codePointAt(0)
        return (cp in 0x4E00..0x9FFF) || (cp in 0x3400..0x4DBF)
    }
}
