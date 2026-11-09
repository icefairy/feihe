package com.feihe.ime.clip

/**
 * 剪贴板历史：**纯逻辑，不碰 Android API**。
 *
 * ## 为什么要单独抽出来
 *
 * 去重、置顶、条数上限、换行怎么存，这些都很容易在边界上写错（复制同一段文字两次
 * 应该只有一条；带换行的代码片段不能把文件格式冲垮）。放进 InputMethodService 里
 * 就只能上真机手点，而错了的表现都是「历史里少了一条 / 多了重复项」，没人会注意。
 *
 * ## 抓取时机的限制（不是 bug）
 *
 * Android 10 起，只有**当前有焦点的 App**（和默认输入法）能读剪贴板，后台监听这条路
 * 是断的。所以历史只在「键盘出现 / 切输入框 / 打开剪贴板面板」这几个时刻抓一次——
 * 用户在别处复制完切回来，那一刻正好被记下。这已经是输入法能做到的全部。
 */
object ClipHistory {

    /** 最多留几条。30 条够翻回一个工作时段，又不至于让文件涨得没边。 */
    const val LIMIT = 30

    /**
     * 单条最长多少字符。超过就不记：复制整篇日志、整段代码进历史的收益极低，
     * 而一条 2 MB 的文本会让每次读写都变慢。
     */
    const val MAX_TEXT = 8000

    data class Entry(val text: String, val at: Long)

    /**
     * 把一段新复制的内容并进历史。
     *
     * - 空白不记（很多 App 会往剪贴板塞空串）；
     * - 和已有某条完全相同 → **挪到最前并刷新时间**，而不是再存一条；
     * - 超过 [limit] 的尾巴丢掉；
     * - 太长（> [MAX_TEXT]）直接不记。
     *
     * 返回新列表；没变化时返回**原列表引用**，调用方可以据此跳过写盘。
     */
    fun merge(old: List<Entry>, text: String, at: Long, limit: Int = LIMIT): List<Entry> {
        val t = text.trim()
        if (t.isEmpty() || t.length > MAX_TEXT) return old
        // 已经在第一条：什么都不用做（时间戳也不必刷新——否则键盘每弹一次就写一次盘）
        if (old.firstOrNull()?.text == t) return old
        val rest = old.filterNot { it.text == t }
        return (listOf(Entry(t, at)) + rest).take(limit)
    }

    /** 序列化成一行一条：`时间戳\t转义后的正文`。 */
    fun encode(list: List<Entry>): String =
        list.joinToString("\n") { "${it.at}\t${escape(it.text)}" }

    /** [encode] 的逆运算。坏行直接跳过——历史读不出来不是错误，丢一条就算了。 */
    fun decode(raw: String): List<Entry> =
        raw.lineSequence()
            .filter { it.isNotBlank() }
            .mapNotNull { line ->
                val tab = line.indexOf('\t')
                if (tab <= 0) return@mapNotNull null
                val at = line.substring(0, tab).toLongOrNull() ?: return@mapNotNull null
                Entry(unescape(line.substring(tab + 1)), at)
            }
            .take(LIMIT)
            .toList()

    /** 正文里的 \ / 换行 / 制表符要转义，否则一条多行文本会把文件格式冲垮。 */
    fun escape(s: String): String = buildString(s.length + 8) {
        for (c in s) {
            when (c) {
                '\\' -> append("\\\\")
                '\n' -> append("\\n")
                '\r' -> append("\\r")
                '\t' -> append("\\t")
                else -> append(c)
            }
        }
    }

    fun unescape(s: String): String = buildString(s.length) {
        var i = 0
        while (i < s.length) {
            val c = s[i]
            if (c != '\\' || i + 1 >= s.length) {
                append(c)
                i++
                continue
            }
            when (s[i + 1]) {
                'n' -> append('\n')
                'r' -> append('\r')
                't' -> append('\t')
                '\\' -> append('\\')
                else -> append(s[i + 1])
            }
            i += 2
        }
    }
}
