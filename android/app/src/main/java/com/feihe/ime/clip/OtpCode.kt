package com.feihe.ime.clip

/**
 * 短信验证码识别：从一段剪贴板文本里取出验证码，并判断当前输入框像不像验证码框。
 *
 * **纯逻辑，不碰 Android API**——这样「把订单号当验证码」「把日期当验证码」这类
 * 误判可以在普通 JVM 单测里钉死，不用上真机收短信。
 *
 * 只做识别，不做任何网络/权限动作：剪贴板由调用方读好了传进来。
 */
object OtpCode {

    /** 关键词。中文短信里几乎总有一个，英文 App 用 verification / OTP。 */
    private val KEYWORDS = listOf(
        "验证码", "校验码", "确认码", "动态码", "激活码",
        "verification code", "verification", "security code", "otp", "code",
    )

    private const val MIN_LEN = 4
    private const val MAX_LEN = 8

    /** 太长的文本不做「没有关键词」的兜底识别：整段文章里出现一个 6 位数字太正常了。 */
    private const val SHORT_TEXT = 40

    /**
     * 取验证码。取不到返回 null。
     *
     * 三条规则，从严到宽：
     * 1. 整条就是 4-8 位数字（用户手动复制的常见形态）；
     * 2. 出现关键词 → 取关键词**后面**最近的一个 4-8 位数字；
     * 3. 整段很短且只有一个 4-8 位数字串 → 认它（很多 App 只复制代码本身，
     *    没有「验证码」字样）。
     *
     * 时间、金额、订单号通常不在 4-8 位纯数字里，或者和关键词不挨着，会被前两条挡掉。
     */
    fun extract(raw: String?): String? {
        if (raw == null) return null
        val text = normalize(raw)
        if (text.isBlank()) return null
        val trimmed = text.trim()

        if (trimmed.length in MIN_LEN..MAX_LEN && trimmed.all { it.isDigit() }) return trimmed

        val lower = text.lowercase()
        val afterKeyword = KEYWORDS
            .mapNotNull { k -> lower.indexOf(k).takeIf { it >= 0 }?.plus(k.length) }
            .minOrNull()
        if (afterKeyword != null) {
            digitRuns(text, afterKeyword).firstOrNull { it.length in MIN_LEN..MAX_LEN }?.let { return it }
        }

        if (text.length <= SHORT_TEXT) {
            val runs = digitRuns(text, 0).filter { it.length in MIN_LEN..MAX_LEN }
            // 只有一个数字串也不一定是验证码：「有效期至 2023 年」里的 2023 会被
            // 整段识别成验证码，然后候选条上多出一个没人点的按钮。
            if (runs.size == 1 && !isYearLike(runs[0])) return runs[0]
        }
        return null
    }

    /** 文本里有没有验证码关键词（给「这个输入框不是数字框」的兜底判断用）。 */
    fun hasKeyword(text: String?): Boolean {
        if (text.isNullOrBlank()) return false
        val lower = text.lowercase()
        return KEYWORDS.any { lower.contains(it) }
    }

    /**
     * 这个输入框像不像验证码框。
     *
     * 数字 / 电话类型直接算——绝大多数验证码框是 `inputType="number"`。
     * 其余看 hint（很多 App 会把「请输入验证码」写进 hintText）。
     *
     * 位掩码写成常量而不是 import `InputType`：那些是 Android 的整型常量，
     * 在 JVM 单测里会变成「Method not mocked」。出处：`android.view.inputmethod.InputType`
     * ——`TYPE_MASK_CLASS = 0x0000000f`、`TYPE_CLASS_NUMBER = 0x2`、`TYPE_CLASS_PHONE = 0x3`。
     */
    fun looksLikeOtpField(inputType: Int, hint: CharSequence?): Boolean {
        val cls = inputType and 0x0000000f
        if (cls == 0x2 || cls == 0x3) return true
        return hasKeyword(hint?.toString())
    }

    /** 全角数字转半角、去掉零宽字符和首尾空白——短信里的「１２３４５６」不能漏。 */
    fun normalize(raw: String): String = buildString(raw.length) {
        for (c in raw) {
            when (c) {
                in '０'..'９' -> append('0' + (c - '０'))
                '\u200b', '\u200c', '\u200d', '\ufeff' -> Unit
                else -> append(c)
            }
        }
    }

    /** 1900–2099 的四位数几乎总是年份，不是验证码。 */
    private fun isYearLike(s: String): Boolean =
        s.length == 4 && s.toIntOrNull()?.let { it in 1900..2099 } == true

    /** 从 [from] 开始，按出现顺序取出所有连续数字串。 */
    private fun digitRuns(text: String, from: Int): List<String> {
        val out = mutableListOf<String>()
        var i = from.coerceIn(0, text.length)
        while (i < text.length) {
            if (!text[i].isDigit()) {
                i++
                continue
            }
            val start = i
            while (i < text.length && text[i].isDigit()) i++
            out.add(text.substring(start, i))
        }
        return out
    }
}
