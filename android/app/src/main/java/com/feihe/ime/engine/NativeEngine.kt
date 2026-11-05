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
     * 全拼 → 小鹤双拼码。`nihao` → `nihc`。
     *
     * 失败返回空串：对「编码」这件事来说，「编不出来」和「出错」的结论一样。
     */
    fun encodePinyin(pinyin: String): String =
        if (ensureLoaded()) runCatching { encodePinyinNative(pinyin) }.getOrElse { "" } else ""

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
     * 按双拼码查候选。空列表就是没有候选（不是错误）。
     *
     * 返回 `List` 而不是拼好的字符串：候选要一个个放进视图，中间再切一次字符串
     * 既费事又容易在分隔符上出错。
     */
    fun lookup(code: String, limit: Int = 9): List<String> {
        if (code.isEmpty() || !ensureLoaded()) return emptyList()
        val raw = runCatching { lookupNative(code, limit) }.getOrElse { return emptyList() }
        return if (raw.isEmpty()) emptyList() else raw.split('\n')
    }

    // ── 原生方法 ────────────────────────────────────────────────────────
    //
    // 方法名 + 包名 + 类名拼起来就是 .so 里的符号名：
    // Java_com_feihe_ime_engine_NativeEngine_<方法名>。
    // 改包名或类名必须同步改 android/jni/src/lib.rs，否则运行期 UnsatisfiedLinkError。

    private external fun initNative(dataDir: String, utcOffsetMinutes: Int)

    private external fun lookupNative(code: String, limit: Int): String

    private external fun coreVersionNative(): String

    private external fun encodePinyinNative(pinyin: String): String

    private external fun isSyllableNative(text: String): Boolean
}
