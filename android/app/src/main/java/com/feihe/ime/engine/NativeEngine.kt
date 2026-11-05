package com.feihe.ime.engine

import android.util.Log

/**
 * Rust 引擎（`libfeihe_android.so`）的入口。
 *
 * 这一层的全部职责是**别让引擎的问题把输入法拖死**：`.so` 没打进包里、ABI 不对、
 * 符号名对不上，都会在第一次调用时抛 `UnsatisfiedLinkError`。如果直接在键盘的
 * 按键回调里调原生方法，那一刻就是整个输入法崩溃、用户正在写的字全丢。
 *
 * 所以：统一出口 + 一次尝试加载 + 失败后静默降级（返回空串，界面按「没有候选」
 * 处理）。日志里留一行，`adb logcat -s FeiheIme` 就能看见到底为什么降级。
 */
object NativeEngine {

    private const val TAG = "FeiheIme"

    /** 加载过一次就不再重试：失败是确定性的，每次按键都 try 一遍纯属浪费。 */
    private var state: LoadState = LoadState.NotTried

    private enum class LoadState { NotTried, Ok, Failed }

    /** 引擎是否可用。界面据此决定「中文态能不能真的出候选」。 */
    val available: Boolean
        get() = ensureLoaded()

    private fun ensureLoaded(): Boolean {
        if (state == LoadState.NotTried) {
            state = try {
                System.loadLibrary("feihe_android")
                LoadState.Ok
            } catch (e: UnsatisfiedLinkError) {
                Log.e(TAG, "加载 libfeihe_android.so 失败：${e.message}")
                LoadState.Failed
            }
        }
        return state == LoadState.Ok
    }

    /** 引擎版本 + 架构，例如 `0.3.11 (aarch64)`。加载失败返回 "未加载"。 */
    fun coreVersion(): String =
        if (ensureLoaded()) runCatching { coreVersionNative() }.getOrElse { "调用失败" } else "未加载"

    /**
     * 键盘上敲的一串全拼 → 小鹤双拼码。`nihao` → `nihc`、`zhongguo` → `vsgo`。
     *
     * 里面是按「最长音节优先」猜的切分，不用你自己加空格。
     * 失败返回空串：还没打完（`nih`）、或者拼了不存在的音节（`zzz`），
     * 对编码这件事来说结论一样——没有结果。
     */
    fun encodePhrase(pinyin: String): String =
        if (ensureLoaded()) runCatching { encodePhraseNative(pinyin) }.getOrElse { "" } else ""

    /** 是不是一个完整合法的拼音音节。引擎没加载时一律 false（保守：不当作音节）。 */
    fun isSyllable(text: String): Boolean =
        if (ensureLoaded()) runCatching { isSyllableNative(text) }.getOrElse { false } else false

    /**
     * 初始化引擎：数据目录 + 时区偏移。**必须在任何查询之前调一次**。
     *
     * 不调的话 core 那边数据目录为空，`paths::dir()` 会直接 panic（这是刻意的：
     * 宁可炸在入口，也不要静默把文件写到 `/` 下面）。
     *
     * `utcOffsetMinutes` 给自定义短语的 `{hour}` 之类变量用——Rust 侧拿不到时区。
     */
    fun init(dataDir: String) {
        if (!ensureLoaded()) return
        val offset = java.util.TimeZone.getDefault().getOffset(System.currentTimeMillis()) / 60_000
        runCatching { initNative(dataDir, offset) }
            .onFailure { Log.e(TAG, "initNative 失败：${it.message}") }
    }

    /**
     * 按键串 → （实际用的码, 候选）。
     *
     * 第一个值是**状态条要显示的那个码**：全拼模式下打 `nihao` 会得到 `nihc`
     * （不相等 → 显示 `nihao→nihc`），双拼模式下原样当码（相等 → 不显示箭头）。
     *
     * 「这串按键是双拼码还是全拼」的判定在 core 里（`dict::lookup_input`），
     * 这里只传 `pinyinMode`。**不要在 Kotlin 里再判一次**：那是第二份真值，
     * 两边一旦分歧，症状是「候选莫名其妙不对」，根本查不到这里。
     *
     * 返回 `("" to emptyList())` 就是「没有码也没有候选」，不是错误。
     */
    fun lookupInput(input: String, pinyinMode: Boolean, limit: Int = 9): Pair<String, List<String>> {
        if (input.isEmpty() || !ensureLoaded()) return "" to emptyList()
        val raw = runCatching { lookupInputNative(input, pinyinMode, limit) }
            .getOrElse { return "" to emptyList() }
        if (raw.isEmpty()) return "" to emptyList()
        // 第一行是码，剩下的是候选。split 不丢空行，但 code 不会是空串（上面判过），
        // 所以 `drop(1)` 之后每一行都是真候选。
        val lines = raw.split('\n')
        return lines[0] to if (lines.size > 1) lines.subList(1, lines.size) else emptyList()
    }

    /**
     * 上一个上屏词后面可能接的词。空列表 = 没有联想（这也意味着候选条不该为它留位置）。
     *
     * 顺序是「你自己打过的优先，内置表补位」，这个顺序在 core 里定死了，界面**不要**
     * 重排——用户自己打过一次的词，对他永远比词库统计更可信。
     */
    fun suggest(prev: String, limit: Int = 4): List<String> {
        if (prev.isEmpty() || !ensureLoaded()) return emptyList()
        val raw = runCatching { suggestNative(prev, limit) }.getOrElse { return emptyList() }
        return if (raw.isEmpty()) emptyList() else raw.split('\n')
    }

    /**
     * 记一对「上次上屏 → 这次上屏」。上屏时调，越频繁越好——写盘节流在 core 里，
     * 这里不用自己攒。
     *
     * `prev` 为空时直接返回：`next::record` 本来也会拒掉，但那要过一次 JNI，
     * 而「本次输入的第一个词」是最常见的情况（每次切窗口后的第一下）。
     */
    fun record(prev: String, next: String) {
        if (prev.isEmpty() || next.isEmpty() || !ensureLoaded()) return
        runCatching { recordNative(prev, next) }
    }

    /**
     * 设置页的开关快照。`null` = 引擎没加载，设置页据此显示「引擎未就绪」而不是
     * 一排点了没反应的开关。
     */
    data class Settings(
        /** `xiaohe` 或 `ms`。 */
        val scheme: String,
        /** true = 全拼模式（不按双拼解释按键）。 */
        val pinyin: Boolean,
        val traditional: Boolean,
        val suggest: Boolean,
        val learn: Boolean,
    )

    fun settings(): Settings? {
        if (!ensureLoaded()) return null
        val raw = runCatching { configNative() }.getOrElse { return null }
        // 格式是 `key=value;key=value`，解析出错时按默认值兜底而不是抛异常：
        // 设置页拿不到值只需要显示默认，没必要崩。
        val map = raw.split(';')
            .mapNotNull { kv ->
                val i = kv.indexOf('=')
                if (i <= 0) null else kv.substring(0, i) to kv.substring(i + 1)
            }
            .toMap()
        return Settings(
            scheme = map["scheme"] ?: "xiaohe",
            pinyin = map["pinyin"] == "1",
            traditional = map["traditional"] == "1",
            suggest = map["suggest"] == "1",
            learn = map["learn"] == "1",
        )
    }

    /** 改一项设置。key 是白名单里的那几个（见 `android/jni/src/lib.rs`）。 */
    fun setSetting(key: String, value: String) {
        if (!ensureLoaded()) return
        runCatching { setConfigNative(key, value) }
    }

    // ── 原生方法 ────────────────────────────────────────────────────────
    //
    // 方法名 + 包名 + 类名拼起来就是 .so 里的符号名：
    // Java_com_feihe_ime_engine_NativeEngine_<方法名>。
    // 改包名或类名必须同步改 android/jni/src/lib.rs，否则运行期 UnsatisfiedLinkError。

    private external fun initNative(dataDir: String, utcOffsetMinutes: Int)

    private external fun lookupInputNative(input: String, pinyinMode: Boolean, limit: Int): String

    private external fun suggestNative(prev: String, limit: Int): String

    private external fun recordNative(prev: String, next: String)

    private external fun configNative(): String

    private external fun setConfigNative(key: String, value: String)

    private external fun coreVersionNative(): String

    private external fun encodePhraseNative(pinyin: String): String

    private external fun isSyllableNative(text: String): Boolean
}
