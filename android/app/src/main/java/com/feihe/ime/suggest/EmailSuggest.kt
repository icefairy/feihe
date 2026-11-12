package com.feihe.ime.suggest

/**
 * 邮箱后缀联想。
 *
 * ## 为什么需要它
 *
 * 输入 QQ 邮箱要打「12345678@qq.com」——数字键盘上 `@` 和 `.` 都得切页去找，
 * 打完一串数字还得切两三次页才能拼出后缀。这是手机上最烦的操作之一。
 *
 * 与其让用户找符号，不如**直接给出整段**：打完 `12345678` 候选条上就出现
 * `@qq.com`、`@163.com`，点一下就补完。
 *
 * ## 为什么是纯对象
 *
 * 触发条件全是边界：数字多长才联想、已经打过 `@` 还联不联想、光标停在句子中间
 * 怎么办、中文语境下会不会突然冒出个邮箱把候选条占满。这些错了的表现都是
 * 「偶尔弹出不合适的东西」，在真机上很难描述、更难复现。抽成纯函数就能用普通
 * JVM 单测把这些边界逐条钉死。
 */
object EmailSuggest {

    /**
     * 光秃秃的一串数字才联想。
     *
     * 上限 15 位：QQ 号最长 11 位，手机号 11 位，留一点余量给其它账号；
     * 再长就是订单号、身份证之类，硬塞邮箱后缀只会干扰。
     */
    const val MIN_DIGITS = 4
    const val MAX_DIGITS = 15

    /** 一句话里最多看前文多少字符。够覆盖「数字 + 一点上下文」，又不至于读进整段。 */
    const val LOOKBEHIND = 32

    /**
     * 常见邮箱后缀，按国内使用频率排序。
     *
     * 顺序有意义：候选条位置有限，越靠前越容易被点到。`qq.com` 排第一是因为
     * 这个功能最初的诉求就是「输完 QQ 号补 @qq.com」。
     */
    val DOMAINS: List<String> = listOf(
        "@qq.com",
        "@163.com",
        "@126.com",
        "@139.com",
        "@189.cn",
        "@sina.com",
        "@foxmail.com",
        "@gmail.com",
        "@outlook.com",
        "@aliyun.com",
    )

    /**
     * 光标前的内容该不该给邮箱联想。
     *
     * @param before 光标前的文字（[LOOKBEHIND] 之内就够）
     * @return 可以联想时返回那串数字，否则返回 null
     *
     * 三种情况必须**不**联想：
     * - 前文末尾不是数字（已经在打英文/中文，或已经打完邮箱了）
     * - 前面已经有 `@`：说明后缀正在（或已经）输入，再联想整段会重复
     * - 数字太短或太长：`123` 可能是验证码的一半，20 位可能是订单号
     */
    fun digitsFor(before: CharSequence?): String? {
        if (before == null) return null
        val s = before.toString()
        if (s.isEmpty()) return null

        // 从尾部往前收连续数字
        var i = s.length
        while (i > 0 && s[i - 1] in '0'..'9') i--
        val digits = s.substring(i)
        if (digits.length < MIN_DIGITS || digits.length > MAX_DIGITS) return null

        // 数字前面紧挨着的东西决定了这串数字是什么身份。
        // 允许：开头、空白、常见标点、**汉字**（「发到12345678」这种）。
        // 不允许：ASCII 字母（abc123 是账号或型号）、`@`（已经在邮箱里）、
        // `.`（1.2.3 是版本号/IP）、`-` `_`（订单号、型号）。
        //
        // 注意别用 Character.isLetterOrDigit：它是 Unicode 语义，汉字也算 letter，
        // 「发到12345678」会被误判成字母开头而漏掉联想——这正是第一版的问题。
        if (i > 0) {
            val prev = s[i - 1]
            val asciiLetter = prev in 'a'..'z' || prev in 'A'..'Z'
            if (asciiLetter) return null
            if (prev == '@' || prev == '.' || prev == '-' || prev == '_') return null
        }
        return digits
    }

    /**
     * 给这串数字配出候选后缀。
     *
     * 返回的每项都是**完整后缀**（含 `@`），点击时直接 commit，不用再拼。
     *
     * @param digits [digitsFor] 的结果
     * @param limit 最多给几条（候选条位置有限）
     */
    fun domainsFor(digits: String?, limit: Int = 4): List<String> {
        if (digits.isNullOrEmpty()) return emptyList()
        if (limit <= 0) return emptyList()
        return DOMAINS.take(limit)
    }
}
