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

    /** 引擎版本 + 架构，例如 `0.3.12 (aarch64)`。加载失败返回 "未加载"。 */
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
    fun lookupInput(input: String, pinyinMode: Boolean, limit: Int = 9): Lookup {
        if (input.isEmpty() || !ensureLoaded()) return Lookup("", emptyList(), emptyList())
        val raw = runCatching { lookupInputNative(input, pinyinMode, limit) }
            .getOrElse { return Lookup("", emptyList(), emptyList()) }
        if (raw.isEmpty()) return Lookup("", emptyList(), emptyList())
        // 第一行是码，剩下的是候选；若有整句，末尾还有 `#seg` 标记行和分段数据
        // （每行 `段码\t候选…`）。分段由 core 在这次查询里顺手算好搭车回来，
        // 壳**不要**再调一次 JNI 重算——那等于同一段 DP 每次按键跑两遍。
        val lines = raw.split('\n')
        val segIdx = lines.indexOf(SEG_MARK)
        val words = if (segIdx >= 0) lines.subList(1, segIdx) else lines.drop(1)
        val segments = if (segIdx >= 0) {
            lines.subList(segIdx + 1, lines.size).mapNotNull { line ->
                val parts = line.split('\t')
                // 至少要有段码 + 一个候选；格式不对的行直接丢掉，别让半截数据进 UI
                if (parts.size < 2) null else SegmentChoice(parts[0], parts.drop(1))
            }
        } else {
            emptyList()
        }
        return Lookup(lines[0], words, segments)
    }

    /** [lookupInput] 的返回值。`segments` 非空 = words[0] 是各段第一名拼成的整句。 */
    data class Lookup(
        val code: String,
        val words: List<String>,
        val segments: List<SegmentChoice>,
    )

    /** 一段的候选（逐段选择 A1）。`code` 只给诊断/状态条用，界面显示的是 [words]。 */
    data class SegmentChoice(val code: String, val words: List<String>)

    /** JNI 返回串里「候选结束、分段数据开始」的标记行。候选和码都不可能以 # 开头。 */
    private const val SEG_MARK = "#seg"

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
        /** 玄学分类词库（干支、易经、奇门、六壬、六爻、神煞），默认关。 */
        val xuanxue: Boolean,
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
            xuanxue = map["xuanxue"] == "1",
        )
    }

    /** 改一项设置。key 是白名单里的那几个（见 `android/jni/src/lib.rs`）。 */
    fun setSetting(key: String, value: String) {
        if (!ensureLoaded()) return
        runCatching { setConfigNative(key, value) }
    }

    // ── 数据备份 ────────────────────────────────────────────────────────
    //
    // 打包/解包在 Kotlin（`java.util.zip`），这里只问「哪些文件算用户数据」和
    // 「导入之后怎么让它生效」。**清单必须来自引擎**：写在 Kotlin 里就是第二份真值，
    // 引擎新增一个数据文件时备份会静默漏掉它。

    /** 允许备份/导入的数据文件名。导入端拿它当白名单（挡路径穿越和任意大文件）。 */
    fun dataFiles(): List<String> {
        if (!ensureLoaded()) return emptyList()
        val raw = runCatching { dataFilesNative() }.getOrElse { return emptyList() }
        return if (raw.isEmpty()) emptyList() else raw.split('\n').filter { it.isNotEmpty() }
    }

    /** 导出包里的 `MANIFEST.txt` 正文（版本 + 时间 + 每个文件的大小）。 */
    fun manifest(): String =
        if (ensureLoaded()) runCatching { manifestNative() }.getOrElse { "" } else ""

    /**
     * 导入之后调：把配置和所有内存表重读一遍。
     *
     * 不调的话「导入成功」是假的——第一次按键用的还是旧词频、旧设置。
     */
    fun reloadAll() {
        if (!ensureLoaded()) return
        runCatching { reloadAllNative() }
    }

    // ── 快捷输入（`/rq` `/sj` …） ───────────────────────────────────────
    //
    // 这一摊的逻辑全在 feihe-core 的 `quick` 模块里，Windows 端用的是同一份。
    // Kotlin 这里只做三件事：问引导符、按码取候选、问「这串码还合不合法」。

    /**
     * 快捷输入的引导符（目前是 `/`）。
     *
     * **不要自己写死**：引导符是引擎的语法，壳里再写一份，哪天引擎改了，
     * Android 就会静默地不再支持快捷输入。
     */
    fun quickPrefix(): String =
        if (ensureLoaded()) runCatching { quickPrefixNative() }.getOrElse { "" } else ""

    /**
     * `/` 后面的码 → 候选（按显示顺序）。`key` 不含引导符。
     *
     * `key` 为空 = 只打了 `/`，返回全部命令的当前值预览（「`/rq` 今天日期」）。
     */
    fun quickCandidates(key: String, limit: Int = 9): List<String> {
        if (!ensureLoaded()) return emptyList()
        val raw = runCatching { quickCandidatesNative(key, limit) }.getOrElse { return emptyList() }
        return if (raw.isEmpty()) emptyList() else raw.split('\n')
    }

    /**
     * 这串码是不是**至少还能往下打**。
     *
     * 必须支持中间态（`/uuid` 会经过 `u`、`uu`），否则用户看到的是「打一半字消失」。
     * 键盘用它决定这一下按键吃不吃进组合串。
     */
    fun quickCodeOk(key: String): Boolean =
        if (ensureLoaded()) runCatching { quickCodeOkNative(key) }.getOrElse { false } else false

    // ── 个人词库（导入 / 导出 / 编译） ──────────────────────────────────
    //
    // 格式两端同一份（`词<TAB>全拼`），所以「电脑上导出的词库」直接能导进手机。
    // 文件选择器给路径，这里只调引擎；返回串是 `"OK …"` 或 `"ERR …"`，
    // 由界面把后半截显示出来——出错时什么都不说是最让人抓狂的。

    /** 导出全部词库到 [path]。返回给用户看的一句话。 */
    fun dictExport(path: String): String =
        if (ensureLoaded()) runCatching { dictExportNative(path) }.getOrElse { "ERR 引擎未就绪" }
        else "ERR 引擎未就绪"

    /** 把 [path] 这份词库并进个人词库（追加，不覆盖）。 */
    fun dictImport(path: String): String =
        if (ensureLoaded()) runCatching { dictImportNative(path) }.getOrElse { "ERR 引擎未就绪" }
        else "ERR 引擎未就绪"

    /** 把 `user_dict.txt` 编译成 `user.bin`。 */
    fun dictCompile(): String =
        if (ensureLoaded()) runCatching { dictCompileNative() }.getOrElse { "ERR 引擎未就绪" }
        else "ERR 引擎未就绪"

    /** 个人词库当前实际生效的来源（`user.bin 已启用` / `文本回退` / `未导入`）。 */
    fun dictSource(): String =
        if (ensureLoaded()) runCatching { dictSourceNative() }.getOrElse { "" } else ""

    /**
     * 从搜狗词库文件导入（`.scel` 细胞词库 / `sgim_gd_usr*.bin` / 导出的 `.txt`）。
     *
     * 和 [dictImport] 的区别是**输入格式**：那边只要「词 + Tab + 全拼」的文本，
     * 这边认的是搜狗自己的二进制格式，词是解析出来之后再自动注音的（注音在
     * `feihe_core::sogou`，和 Windows 端共用同一份）。
     *
     * 返回 [SogouImport]：成功给一段多行摘要和新增条数，失败给原因。
     */
    fun sogouImport(path: String): SogouImport {
        val raw = if (ensureLoaded()) {
            runCatching { sogouImportNative(path) }.getOrElse { "ERR|引擎未就绪" }
        } else {
            "ERR|引擎未就绪"
        }
        // 引擎返回 `"OK|<多行摘要>|<新增条数>"` 或 `"ERR|<原因>"`。
        // 摘要里本来就有换行，所以只能用第一个 `|` 和最后一个 `|` 切。
        val parts = raw.split('|')
        return when (parts.firstOrNull()) {
            "OK" -> {
                // 末尾那个字段是新增条数；不是数字就当摘要本身（防止老 .so 少给字段）
                val added = parts.last().trim().toIntOrNull()
                val body = if (added == null) parts.drop(1) else parts.subList(1, parts.size - 1)
                SogouImport(true, body.joinToString("|"), added ?: 0)
            }
            "ERR" -> SogouImport(false, parts.drop(1).joinToString("|"), 0)
            // 引擎没按约定返回（老版本 .so）：原样显示，总比吞掉强
            else -> SogouImport(false, raw, 0)
        }
    }

    /** [sogouImport] 的结果。 */
    data class SogouImport(val ok: Boolean, val message: String, val added: Int)

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

    private external fun dataFilesNative(): String

    private external fun manifestNative(): String

    private external fun reloadAllNative()

    private external fun coreVersionNative(): String

    private external fun encodePhraseNative(pinyin: String): String

    private external fun isSyllableNative(text: String): Boolean

    private external fun quickPrefixNative(): String

    private external fun quickCandidatesNative(key: String, limit: Int): String

    private external fun quickCodeOkNative(key: String): Boolean

    private external fun dictExportNative(path: String): String

    private external fun dictImportNative(path: String): String

    private external fun dictCompileNative(): String

    private external fun dictSourceNative(): String

    private external fun sogouImportNative(path: String): String
}
