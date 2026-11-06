package com.feihe.ime.suggest

/**
 * 联想的上下文来源 + 候选条容量分配。
 *
 * ## 为什么单独抽成纯 Kotlin
 *
 * 这两块都要在**普通 JVM 单测**里跑：读宿主前文涉及「密码框绝对不能读」「末尾是
 * 英文就别硬凑中文词」这类边界，候选让位涉及「给联想留几个位置」的算术。放进
 * `InputMethodService` 就只能上真机靠手点，而错了的表现都是「偶尔不对」。
 */

/**
 * 从宿主输入框光标前的文字里取「上一个词」，用来给联想做上下文。
 *
 * 以前只在**同一段连续输入**里记词对：打完「北京」接着打「时间」，`record` 能学到
 * 这一对；但切到微信、或者光标停在已有文字后面，第一个词就没有上文可学。
 * 搜狗这类输入法会从 `getTextBeforeCursor` 拿前文，这里补上。
 */
object HostContext {
    /** 一个词最多取末尾几个汉字。词对表按整词索引，取太长反而匹配不到。 */
    const val WORD_CHARS = 2

    /**
     * 能不能读这个输入框的前文。
     *
     * **密码框一律不读**——这是底线，项目 Windows 端「密码框一个字都不接管」是同一条
     * 规矩。另外尊重宿主的 `FLAG_NO_SUGGESTIONS`（搜索框、验证码框常带）。
     *
     * 这里故意不 `import android.view.inputmethod.InputType`：那些是 `const val` 整型
     * 常量，在 JVM 单测里会变成「Method not mocked」的坑。把位掩码抄成局部常量并注明
     * 出处，换来的是这段安全判断能被单测钉死——比少写四行常量重要。
     *
     * 出处：`android.view.inputmethod.InputType`。
     */
    fun canRead(inputType: Int): Boolean {
        if (inputType == 0) return false // TYPE_NULL
        // TYPE_TEXT_FLAG_NO_SUGGESTIONS = 0x00080000
        if (inputType and 0x00080000 != 0) return false
        // TYPE_MASK_VARIATION = 0x00000ff0；下面是所有「密码类」variation
        val variation = inputType and 0x00000ff0
        return when (variation) {
            0x00000080, // TYPE_TEXT_VARIATION_PASSWORD
            0x00000090, // TYPE_TEXT_VARIATION_VISIBLE_PASSWORD
            0x000000e0, // TYPE_TEXT_VARIATION_WEB_PASSWORD
            0x00000010, // TYPE_NUMBER_VARIATION_PASSWORD
            -> false
            else -> true
        }
    }

    /**
     * 取光标前的「上一个词」：末尾连续汉字，最多 [WORD_CHARS] 个。
     *
     * - 末尾是标点/空白 → 先剥掉再取（「我今天去了北京，」→「北京」）。
     * - 末尾是英文字母或数字 → 返回空串。**不要跨过英文去捞更前面的汉字**：
     *   「发我 abc」的上一词是 abc，拿「我」去联想只会给出不相干的词。
     * - 没有汉字 → 空串（调用方按「没有上下文」处理）。
     */
    fun lastWord(before: CharSequence?): String {
        if (before == null) return ""
        val s = before.toString()
        var i = s.length
        // 1) 剥掉尾部空白和标点（非汉字、非字母、非数字都算分隔符）
        while (i > 0 && isSeparator(s[i - 1])) i--
        if (i == 0) return ""
        // 2) 末尾是 ASCII 字母/数字 → 这是英文/数字串，不猜中文词
        if (isAsciiWord(s[i - 1])) return ""
        // 3) 往回收集汉字
        val start = i
        while (i > 0 && isCjk(s[i - 1]) && start - i < WORD_CHARS) i--
        if (i == start) return ""
        return s.substring(i, start)
    }

    private fun isCjk(c: Char): Boolean {
        val v = c.code
        // CJK 统一表意文字基本区 + 扩展 A；足够覆盖日常用字
        return v in 0x4E00..0x9FFF || v in 0x3400..0x4DBF
    }

    private fun isAsciiWord(c: Char): Boolean =
        c in 'a'..'z' || c in 'A'..'Z' || c in '0'..'9'

    /** 分隔符：汉字/字母/数字以外的都算（标点、空白、emoji 代理项）。 */
    private fun isSeparator(c: Char): Boolean =
        !isCjk(c) && !isAsciiWord(c)
}

/**
 * 候选条容量：候选和联想共用一行横向滚动条时怎么分配。
 *
 * 以前固定给 9 个候选，联想 `→词` 排在后面，候选一满就得横滑才看得见联想。
 * 改成**有联想时把候选压到 6 个**，给联想留出位置；没有联想时按用户设的上限给满。
 */
object CandidateBarPolicy {
    /** 候选上限的可选档位。单测钉住，防止设置页和读取端各写一份。 */
    val LIMITS = listOf(5, 7, 9)

    const val DEFAULT_LIMIT = 9

    /** 有联想时最多给几个候选。留出 `→词` 的位置（SUGGEST_LIMIT 是 5，横滑一点即可）。 */
    const val RESERVED_FOR_SUGGESTIONS = 6

    /** 设置值不合法（手改过 SharedPreferences）时回落到默认档。 */
    fun sanitize(limit: Int): Int = if (limit in LIMITS) limit else DEFAULT_LIMIT

    /** 本次实际渲染几个候选。 */
    fun candidateSlots(limit: Int, hasSuggestions: Boolean): Int {
        val n = sanitize(limit)
        return if (hasSuggestions) minOf(n, RESERVED_FOR_SUGGESTIONS) else n
    }
}
