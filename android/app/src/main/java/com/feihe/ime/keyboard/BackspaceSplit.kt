package com.feihe.ime.keyboard

/**
 * 退格时「该不该把整词切成逐字」，判据和 Windows 端 `textservice.rs::backspace_splits`
 * **完全一致**——两端手感必须一样，用户换设备不该重新学一遍。
 *
 * ## 它解决什么
 *
 * 打 `xrji` 出「璇玑」，用户想要的却是「璇」+ 别的字。品牌输入法的做法是**按退格**：
 * 编码不动，候选从「整词」变成 `[璇][玑]` 两段，各段自己换字。
 *
 * 注意这**不是**「删掉最后一个音节」——编码一个字都不少。以前写成删音节是错的：
 * 用户退一格是想换字，结果编码被吃掉一截，还得重新打。
 *
 * ## 为什么不拆单字码
 *
 * `de` → 「的」这种码本身就是「一堆同码字」，第一候选是单字。这时候「拆成逐字」
 * 毫无意义（拆出来还是同一个字），退格该老实删字母。
 *
 * ## 为什么抽成纯函数
 *
 * 判据只有三个分支，但错了的表现很隐蔽：拆早了，用户打一半就被劫持；拆晚了，
 * 该换字时却在删字母。真机验证要装包、打字、按退格、看候选，每改一次来一遍。
 * 抽出来就能穷举单测。
 */
object BackspaceSplit {

    /**
     * @param candidates 当前候选列表（顺序即词频顺序）
     * @return true = 应该切成逐字，编码不动
     */
    fun shouldSplit(candidates: List<String>): Boolean {
        val first = candidates.firstOrNull() ?: return false
        // 单字不拆：用户要的是换候选，不是换字
        return isWord(first)
    }

    /**
     * 是不是「一个词」而不是「一个单字」。
     *
     * 两个条件缺一不可：长度 ≥ 2，且不含 ASCII 字母数字。
     * 后半条是为了排除「候选其实是原始编码」（`nihao` 这种没命中时的退化显示）。
     */
    fun isWord(text: String): Boolean =
        text.length >= 2 && text.none { it.isAsciiLetterOrDigit() }

    private fun Char.isAsciiLetterOrDigit(): Boolean =
        this in 'a'..'z' || this in 'A'..'Z' || this in '0'..'9'
}
