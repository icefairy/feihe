package com.feihe.ime.voice

/**
 * 语音识别结果的「句末补标点」。
 *
 * ## 为什么需要它
 *
 * 第一版内置的是 `paraformer-zh-small`（79 MB，见 `docs/Android-方案.md` §4.1）。
 * 那个模型的输出是**连续无标点字符串**——官方示例：
 *
 * ```
 * 对我做了介绍啊那么我想说的是呢大家如果对我的研究感兴趣呢
 * ```
 *
 * 语音上屏进输入框之后，用户面对的是一整串没有断句的汉字，还得自己回去补标点。
 * 这一步就是把「至少句末有个句号」补上，让结果能直接发出去。
 *
 * ## 设计原则：宁可不加，也不要加错
 *
 * 标点错了比没有标点更烦人——用户会开始怀疑输入法在乱改自己的话。所以这里的
 * 规则**非常保守**：
 *
 * - 只加**句末**一个字符，中间一个逗号都不猜（猜逗号要理解语义，做不到）。
 * - 已经有标点就一个字都不动（幂等，重复调用安全）。
 * - 只有「吗」判疑问句，用「？」。**「呢」不判**——"我在家呢" 是陈述句，
 *   判成问号是错的；"你呢" 判成句号只是不够好，不是错。两害相权取其轻。
 * - 单字不加（"好" → "好。" 很奇怪）。
 *
 * ## 以后能不能换成小模型
 *
 * 能做，但现成的都不小：sherpa-onnx 唯一的中文标点模型是 CT-Transformer，
 * fp32 **280.7 MB**，int8 版本在 ModelScope 上没有。想用小模型只能自己训一个
 * 字符级的（1–3 MB 量级），这需要标点语料，本机网络下不动。见方案文档 §4.6。
 */
object VoicePunctuator {

    /**
     * 「已经算有标点」的字符。命中就整个跳过。
     *
     * 放进来的是**句末可能出现**的那些：句号问号叹号省略号分号冒号顿号逗号，
     * 以及各种收尾的括号引号。收尾引号尤其重要——`他问：“你好”` 不能再补句号。
     */
    private const val ENDINGS = "。！？!?.…；;：:、，,）)】》」』\"'”’～~"

    /** 有没有汉字。有汉字走中文标点，纯 ASCII 走英文标点。 */
    private val CJK = Regex("[\\u3400-\\u4dbf\\u4e00-\\u9fff]")

    /**
     * 给一句话补句末标点。已经合规的原样返回。
     *
     * 纯函数：不碰任何 Android API，所以能在普通 JVM 单测里跑。
     */
    fun punctuate(raw: String): String {
        val text = raw.trimEnd()
        if (text.isEmpty()) return text

        // 已经有标点：一个字符都不动。这条让函数幂等，重复调用不会变成「。。」
        if (ENDINGS.indexOf(text.last()) >= 0) return text

        // 单字不加。「好」「嗯」后面跟句号在聊天里显得很怪。
        if (text.length == 1) return text

        val question = isQuestion(text)
        return text + when {
            CJK.containsMatchIn(text) -> if (question) "？" else "。"
            else -> if (question) "?" else "."
        }
    }

    /**
     * 是不是疑问句。
     *
     * 只认「吗」：它是疑问句最硬的标志，几乎不会出现在陈述句末。
     * 别的都太暧昧——「呢」在陈述句里很常见（"我在家呢"），「吧」是祈使
     * （"走吧"），疑问代词（什么/怎么）一般在句中不在句末。
     */
    private fun isQuestion(text: String): Boolean = text.endsWith("吗")
}
