package com.feihe.ime.engine

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.view.textservice.SentenceSuggestionsInfo
import android.view.textservice.SpellCheckerSession
import android.view.textservice.SuggestionsInfo
import android.view.textservice.TextInfo
import android.view.textservice.TextServicesManager

/**
 * 英文单词建议：走系统自带的拼写检查服务。
 *
 * ## 为什么借系统的，而不是自己塞一份词库
 *
 * 英文联想必须有几十万词的词典才像样（常用词 5 万 + 词形变化），
 * 打进 APK 要 3~8MB，而且永远追不上新词。而 Android 从 API 14 起就有
 * `SpellCheckerSession`，各大 ROM 和 Gboard 都装了词库，**一次调用零成本**。
 *
 * 代价是：建议质量取决于系统装了哪个拼写检查器（有的 ROM 可能没有）。
 * 所以这里必须**优雅降级**——拿不到建议就返回空，用户照样能靠空格确认
 * 自己打的原文，功能不会挂。
 *
 * ## 为什么是异步
 *
 * `getSuggestions` 是跨进程调用（要连系统的 spell checker service），
 * 慢的时候几十毫秒。必须回调式，绝不能同步等——那会卡住键盘输入。
 */
class EnglishSuggest(private val ctx: Context) {

    /** 回调可能来自任意线程，统一回主线程。 */
    private val main = Handler(Looper.getMainLooper())

    private var session: SpellCheckerSession? = null

    /**
     * 当前请求的序号。
     *
     * 用户连打字母时会连发多次查询，而系统回调**可能乱序**（后发的先回）。
     * 没有序号就会出现「打了 hel 却显示 he 的建议」。回调里对不上最新序号就丢弃。
     */
    @Volatile
    private var seq = 0

    /** 当前请求对应的前缀，用来在回调里再校验一次。 */
    @Volatile
    private var pendingWord = ""

    private val listener = object : SpellCheckerSession.SpellCheckerSessionListener {
        override fun onGetSuggestions(results: Array<out SuggestionsInfo>?) {
            // 单词级 API 不用，全部走句子级（它能保留原始前缀、方便校验）
        }

        override fun onGetSentenceSuggestions(results: Array<out SentenceSuggestionsInfo>?) {
            val mySeq = seq
            val want = pendingWord
            if (want.isEmpty()) return
            val info = results?.firstOrNull() ?: run {
                main.post { if (mySeq == seq) onResult?.invoke(want, emptyList()) }
                return
            }
            val out = ArrayList<String>(info.suggestionsCount)
            for (i in 0 until info.suggestionsCount) {
                val s = info.getSuggestionsInfoAt(i) ?: continue
                for (j in 0 until s.suggestionsCount) {
                    val w = s.getSuggestionAt(j) ?: continue
                    if (w.isNotEmpty() && w !in out) out.add(w)
                }
            }
            main.post {
                // 期间用户又打了字母（seq 变了）→ 这批结果已经过时，丢掉
                if (mySeq == seq) onResult?.invoke(want, out)
            }
        }
    }

    /** 结果回调：`(原先输入的词, 建议列表)`。在主线程调用。 */
    var onResult: ((String, List<String>) -> Unit)? = null

    private fun ensureSession(): SpellCheckerSession? {
        session?.let { return it }
        val tsm = ctx.getSystemService(Context.TEXT_SERVICES_MANAGER_SERVICE)
            as? TextServicesManager ?: return null
        return runCatching {
            tsm.newSpellCheckerSession(null, null, listener, true)
        }.getOrNull()?.also { session = it }
    }

    /**
     * 查一个前缀的建议。
     *
     * `prefix` 就是用户已经打出来的字母（`hel`）。系统会返回 hello / help 之类。
     * 传空串等于取消，避免上一次的异步结果落到新会话里。
     */
    fun query(prefix: String) {
        if (prefix.isEmpty()) {
            pendingWord = ""
            seq++
            return
        }
        val s = ensureSession() ?: run {
            // 没有拼写检查服务（部分精简 ROM）：直接给空，不报错不打扰用户
            onResult?.invoke(prefix, emptyList())
            return
        }
        seq++
        pendingWord = prefix
        runCatching {
            // 句子级 API：能拿到原始词，且支持「部分输入」的前缀补全
            s.getSentenceSuggestions(arrayOf(TextInfo(prefix)), SUGGEST_LIMIT)
        }.onFailure {
            onResult?.invoke(prefix, emptyList())
        }
    }

    fun close() {
        runCatching { session?.close() }
        session = null
    }

    private companion object {
        /** 最多要几条建议。候选条一屏放得下 5~7 个，多要没意义。 */
        const val SUGGEST_LIMIT = 8
    }
}
