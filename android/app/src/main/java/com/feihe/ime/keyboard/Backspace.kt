package com.feihe.ime.keyboard

/**
 * 退格要删几个 UTF-16 code unit。
 *
 * ## 为什么不能固定删 1
 *
 * `deleteSurroundingText(1, 0)` 删的是**一个 code unit**，不是一个「字符」。
 * emoji（😊 U+1F60A）、部分生僻汉字、旗子、带肤色修饰的符号都是**代理对**，
 * 占两个 code unit。固定删 1 只会切掉后半个代理项，留下一个孤立的高位代理——
 * 屏幕上显示成乱码方块，用户得再按一次删除才干净。
 *
 * 这就是「emoji 退格只能删一半」的全部原因。
 *
 * ## 为什么单独抽出来
 *
 * 这段要能跑普通 JVM 单测：只依赖字符串本身，不需要 InputConnection。
 * 真机上验证这条要装包、切键盘、打 emoji、按退格、看输入框——一整套下来
 * 每改一次都要重来一遍，而错了的表现（残留半个字符）还很难截图说明。
 */
object Backspace {

    /**
     * 按光标前的文本算出该删几个 code unit。
     *
     * @param beforeCursor 光标前的文字，只需要末尾几个字符
     * @return 1（普通字符）或 2（代理对）；读不到内容时返回 1（保守：宁可删少了
     *         让用户再按一次，也不能一次删掉两个真字）
     */
    fun unitsToDelete(beforeCursor: CharSequence?): Int {
        if (beforeCursor.isNullOrEmpty()) return 1
        val s = beforeCursor.toString()
        val last = s[s.length - 1]
        // 末尾是低位代理项，且前一个字符是高位代理项 → 这是完整的代理对，删 2
        if (Character.isLowSurrogate(last) &&
            s.length >= 2 &&
            Character.isHighSurrogate(s[s.length - 2])
        ) {
            return 2
        }
        return 1
    }

    /** 只给了末尾两个字符时的便捷重载（读整段前文没必要）。 */
    fun unitsToDelete(lastChar: Char, prevChar: Char?): Int =
        if (Character.isLowSurrogate(lastChar) &&
            prevChar != null &&
            Character.isHighSurrogate(prevChar)
        ) {
            2
        } else {
            1
        }
}
