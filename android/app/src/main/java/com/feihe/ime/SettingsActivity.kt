package com.feihe.ime

import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.graphics.drawable.GradientDrawable
import android.net.Uri
import android.os.Bundle
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.Switch
import android.widget.TextView
import android.widget.Toast
import com.feihe.ime.engine.NativeEngine
import java.io.File

/**
 * 设置页。
 *
 * ## 三个入口，缺一个都会有人找不到
 *
 * 1. **键盘状态条右侧的齿轮** —— 打字时最顺手，用户第一次装完就在这儿；
 * 2. **系统设置 → 输入法 → 飞鹤 → 设置** —— 走 `service` 的
 *    `android:settingsActivity` 属性（见 `AndroidManifest.xml`），系统自己会读；
 * 3. **桌面图标** —— 装在「飞鹤输入法」名下，从应用列表也能进。
 *
 * 三者指向同一个 Activity。**只有一个入口是不够的**：输入法不像普通 App，
 * 用户装完不会去桌面找它，也不会想到「设置里还有一个输入法子菜单」。
 *
 * ## 为什么用代码搭 UI 而不是 XML
 *
 * 这一页只有「几行开关 + 两行单选」，XML 要三份文件（layout / 两个 selector），
 * 而且改一个间距要跳两个文件。和 `KeyboardView` 同一个理由：真值在代码里，
 * 少一份要同步的东西。
 *
 * ## 引擎没加载时
 *
 * 从桌面进入时输入法进程可能还没起，`NativeEngine.init` 会在这里补一次。
 * 如果 `.so` 本身加载失败，`settings()` 返回 `null`，页面照常显示但开关全部禁用，
 * 顶部给一句「引擎未就绪」——**不要假装能改**，改了写不进去比不给改更糟。
 */
class SettingsActivity : Activity() {

    private var cfg: NativeEngine.Settings? = null
    private var schemeBox: LinearLayout? = null
    private var inputBox: LinearLayout? = null
    private var candidateBox: LinearLayout? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // 从桌面图标进来时输入法服务没起过，数据目录也还没注册。
        // `paths::init` 是 OnceLock，从输入法那边已经注册过的话这次是空操作。
        NativeEngine.init(filesDir.absolutePath)
        cfg = NativeEngine.settings()
        setContentView(buildUi())
    }

    // ── 页面骨架 ────────────────────────────────────────────────────

    private fun buildUi(): View {
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(BG)
        }

        root.addView(
            TextView(this).apply {
                text = "飞鹤输入法"
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 22f)
                setTextColor(TEXT)
                setPadding(dp(20f), dp(24f), dp(20f), dp(2f))
            },
        )
        root.addView(
            TextView(this).apply {
                text = if (cfg == null) {
                    "引擎未就绪，设置项暂时不可改"
                } else {
                    "引擎 ${NativeEngine.coreVersion()}　·　改动立即生效"
                }
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 13f)
                setTextColor(if (cfg == null) DANGER else SUB)
                setPadding(dp(20f), 0, dp(20f), dp(12f))
            },
        )

        val list = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(12f), 0, dp(12f), dp(24f))
        }

        // 输入方式放在最上面：它解释的是「为什么我打 xian 出了西安」，比方案更常被问
        inputBox = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        list.addView(card("输入方式", inputBox!!))
        renderInputMode()

        // 输入方案（只有双拼模式用得上，但不隐藏：切了模式才发现选错了更糟）
        schemeBox = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        list.addView(card("双拼方案", schemeBox!!))
        renderScheme()

        // 候选个数：手机候选条是单行横向滚动，"行数"在这里就是"一次给几个"。
        // 纯界面开关（UiPrefs），引擎没加载也能改，所以 alwaysEnabled。
        candidateBox = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        list.addView(card("候选个数", candidateBox!!))
        renderCandidateLimit()

        list.addView(
            card(
                "输出",
                switchRow("输出繁体", "上屏时把简体转成繁体", cfg?.traditional == true) {
                    set("traditional", it)
                },
            ),
        )

        list.addView(
            card(
                "学习",
                switchRow("上屏后联想", "在候选条末尾给出下一个词", cfg?.suggest == true) {
                    set("suggest", it)
                },
                switchRow("记住常用词", "常打的词排到前面", cfg?.learn == true) {
                    set("learn", it)
                },
                // 玄学词库：一整摊术数词（干支、易经、奇门、六壬、六爻、神煞）做成
                // 一个开关，而不是分类列表——只有一类用户会开，拆细只会让普通用户
                // 不知道该不该勾。默认关：这些字对多数人只增加短码重码。
                switchRow(
                    "玄学词库",
                    "干支、易经、奇门、六壬、六爻、神煞（默认关，开着短码会多）",
                    cfg?.xuanxue == true,
                ) {
                    set("xuanxue", it)
                    cfg = cfg?.copy(xuanxue = it)
                },
            ),
        )

        // 键盘 / 语音：这两项是手机壳自己的呈现方式，开关存在 SharedPreferences
        // 而不是引擎的 config.ini（桌面端没有对应概念）。引擎没加载也能改，
        // 所以走 alwaysEnabled。
        list.addView(
            card(
                "键盘与语音",
                switchRow(
                    "数字选词行",
                    "有候选时显示 1–9，点数字选词（关掉后不占键盘高度）",
                    UiPrefs.showNumberRow(this),
                    alwaysEnabled = true,
                ) { UiPrefs.setShowNumberRow(this, it) },
                switchRow(
                    "语音输入",
                    "按住状态条上的麦克风说话；关闭后不申请录音权限",
                    UiPrefs.voiceEnabled(this),
                    alwaysEnabled = true,
                ) { on ->
                    UiPrefs.setVoiceEnabled(this, on)
                    // 权限只在这里申请：InputMethodService 不是 Activity，
                    // 从输入法进程弹系统授权框会被拒。打开设置页时顺手要一次。
                    if (on) requestMicPermissionIfNeeded()
                },
                actionRow("语音模型", "模型状态、隐私说明见下方") { showVoiceInfo() },
            ),
        )

        list.addView(
            card(
                "数据",
                actionRow("导出备份", "设置、个人词库、词频、自造词、联想打包成 ${Backup.suggestedName()}") {
                    exportBackup()
                },
                actionRow("导入备份", "用备份里的数据覆盖当前数据（导入前会再确认一次）") {
                    importBackup()
                },
            ),
        )

        // 个人词库单独一个卡片，不和整包备份混在一起：常来这一页的人有一半
        // 就是想「把搜狗词库导进来」或者「把手机上的词带走」，塞在备份里
        // 意味着他要先理解 zip 是什么。
        val dictBox = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        dictBox.addView(
            TextView(this).apply {
                // 引擎没加载时给空串，而不是「未导入」——这两件事必须分清
                text = "当前：" + NativeEngine.dictSource()
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 12f)
                setTextColor(SUB)
                setPadding(dp(14f), dp(2f), dp(14f), dp(6f))
            },
        )
        dictBox.addView(
            actionRow(
                "导入词库",
                "文本格式，每行「词 + Tab + 全拼」；搜狗导出和电脑端导出都是这个格式",
            ) { dictImport() },
        )
        dictBox.addView(
            actionRow("导出词库", "把全部词库导出成文本，可带回电脑端用") { dictExport() },
        )
        dictBox.addView(
            actionRow("从搜狗导入", "下载来的 .scel 细胞词库，或者搜狗导出的 .bin / .txt") {
                importSogouDict()
            },
        )
        dictBox.addView(
            actionRow(
                "编译个人词库",
                "把导入的文本编译成 user.bin，让查询走映射而不是文本回退",
            ) { runBusy { NativeEngine.dictCompile() } },
        )
        list.addView(card("个人词库", dictBox))

        val scroll = ScrollView(this).apply { addView(list) }
        root.addView(
            scroll,
            LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f),
        )
        return root
    }

    private fun card(title: String, vararg rows: View): View {
        val box = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            background = GradientDrawable().apply {
                cornerRadius = dp(10f).toFloat()
                setColor(CARD)
            }
            setPadding(0, dp(6f), 0, dp(6f))
        }
        box.addView(
            TextView(this).apply {
                text = title
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 12f)
                setTextColor(SUB)
                setPadding(dp(14f), dp(6f), dp(14f), dp(2f))
            },
        )
        rows.forEach { box.addView(it) }
        // card 之间留白：紧贴着会看成一张大卡片
        box.layoutParams = LinearLayout.LayoutParams(
            LinearLayout.LayoutParams.MATCH_PARENT,
            LinearLayout.LayoutParams.WRAP_CONTENT,
        ).apply { bottomMargin = dp(12f) }
        return box
    }

    private fun switchRow(
        label: String,
        desc: String,
        checked: Boolean,
        // 纯界面开关不依赖引擎：引擎没加载时用户也应该能关掉数字选词行。
        alwaysEnabled: Boolean = false,
        onToggle: (Boolean) -> Unit,
    ): View {
        val row = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(dp(14f), dp(8f), dp(10f), dp(8f))
        }
        row.addView(
            textBlock(label, desc),
            LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f),
        )
        row.addView(
            Switch(this).apply {
                isChecked = checked
                isEnabled = cfg != null || alwaysEnabled
                // 先 setChecked 再挂监听：反过来的话上面那行会触发一次「刚改成这个值」，
                // 每次进设置页都白写一遍配置。
                setOnCheckedChangeListener { _, v -> onToggle(v) }
            },
        )
        return row
    }

    /**
     * 输入方式：双拼 / 全拼。
     *
     * 对应的就是 `config.ini` 的 `pinyin_mode`，**和 Windows 端同一个字段**——
     * 不另开一个 Android 专用键，那样两端的「我现在是哪种模式」就会各说一套。
     *
     * 两个文案都写清楚「打什么」，因为用户分不清「双拼模式」和「双拼方案」的区别：
     * 方式 = 你按的键是什么，方案 = 双拼码怎么排（小鹤 / 微软）。
     */
    private fun renderInputMode() {
        val box = inputBox ?: return
        box.removeAllViews()
        val pinyin = cfg?.pinyin == true
        val enabled = cfg != null
        box.addView(
            choiceRow(
                "双拼（默认）",
                !pinyin,
                enabled,
                "打 nihc 出「你好」。查不到时也认全拼（nihao 一样出「你好」）",
            ) {
                set("pinyin", "0")
                cfg = cfg?.copy(pinyin = false)
                renderInputMode()
            },
        )
        box.addView(
            choiceRow(
                "全拼",
                pinyin,
                enabled,
                "一律按全拼解释：xian 出「先」，不是「西安」",
            ) {
                set("pinyin", "1")
                cfg = cfg?.copy(pinyin = true)
                renderInputMode()
            },
        )
    }

    /**
     * 候选个数（5 / 7 / 9）。
     *
     * 手机候选条是单行横向滚动，桌面端那种"候选行数"在这儿没有对应物，用户真正能
     * 调的是**一次给几个**。单独说明"有联想时会自动少给几个"，否则用户会以为设了 9
     * 却只看到 6 个是 bug —— 那是 [com.feihe.ime.suggest.CandidateBarPolicy] 在给
     * 联想让位。
     */
    private fun renderCandidateLimit() {
        val box = candidateBox ?: return
        box.removeAllViews()
        val cur = UiPrefs.candidateLimit(this)
        com.feihe.ime.suggest.CandidateBarPolicy.LIMITS.forEach { n ->
            box.addView(
                choiceRow(
                    "$n 个",
                    cur == n,
                    true,
                    if (n == 9) "默认。有联想时仍会自动压到 6 个，给 →词 留位置" else null,
                ) {
                    UiPrefs.setCandidateLimit(this, n)
                    renderCandidateLimit()
                },
            )
        }
    }

    private fun renderScheme() {
        val box = schemeBox ?: return
        box.removeAllViews()
        val cur = cfg?.scheme ?: "xiaohe"
        listOf("xiaohe" to "小鹤双拼", "ms" to "微软双拼").forEach { (key, label) ->
            box.addView(
                choiceRow(label, cur == key, cfg != null) {
                    set("scheme", key)
                    cfg = cfg?.copy(scheme = key)
                    renderScheme()
                },
            )
        }
    }

    private fun choiceRow(
        label: String,
        selected: Boolean,
        enabled: Boolean,
        desc: String? = null,
        onClick: () -> Unit,
    ): View {
        val row = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(dp(14f), dp(10f), dp(14f), dp(10f))
            isEnabled = enabled
            setOnClickListener { if (enabled) onClick() }
        }
        val texts = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        texts.addView(
            TextView(this).apply {
                text = label
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 15f)
                setTextColor(if (selected) ACCENT else TEXT)
            },
        )
        if (desc != null) {
            texts.addView(
                TextView(this).apply {
                    text = desc
                    setTextSize(TypedValue.COMPLEX_UNIT_SP, 12f)
                    setTextColor(SUB)
                },
            )
        }
        row.addView(
            texts,
            LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f),
        )
        row.addView(
            TextView(this).apply {
                text = if (selected) "✓" else ""
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 16f)
                setTextColor(ACCENT)
                setPadding(dp(8f), 0, dp(4f), 0)
            },
        )
        return row
    }

    /**
     * 一行「点一下就有事发生」的条目（右侧一个 `›`）。
     *
     * 和 [choiceRow] 分开：那个是单选，右边是勾；这个是动作，右边是箭头。两者混用
     * 会让人以为「导出备份」是个选中状态。
     */
    private fun actionRow(label: String, desc: String, onClick: () -> Unit): View {
        val row = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(dp(14f), dp(10f), dp(14f), dp(10f))
            // 引擎没就绪时禁用：备份的清单来自引擎，点下去只会弹「引擎未就绪」
            isEnabled = cfg != null
            alpha = if (cfg != null) 1f else 0.4f
            setOnClickListener { if (cfg != null) onClick() }
        }
        val texts = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        texts.addView(
            TextView(this).apply {
                text = label
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 15f)
                setTextColor(TEXT)
            },
        )
        texts.addView(
            TextView(this).apply {
                text = desc
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 12f)
                setTextColor(SUB)
            },
        )
        row.addView(texts, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
        row.addView(
            TextView(this).apply {
                text = "›"
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 18f)
                setTextColor(SUB)
                setPadding(dp(8f), 0, dp(2f), 0)
            },
        )
        return row
    }

    // ── 备份：导出 / 导入 ───────────────────────────────────────────────
    //
    // 走系统的文件选择器（SAF）而不是自己拼路径：不用申请存储权限，用户还能自己
    // 决定存到哪（下载目录、SD 卡、网盘 App 的同步目录都行）。代价是只能拿到一个
    // Uri，读写都通过 `contentResolver`——这也正是 [Backup] 那两个函数的签名。

    @Suppress("DEPRECATION")
    private fun exportBackup() {
        val i = Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "application/zip"
            putExtra(Intent.EXTRA_TITLE, Backup.suggestedName())
        }
        runCatching { startActivityForResult(i, REQ_EXPORT) }
            .onFailure { toast("这台设备上没有能找到的文件管理器") }
    }

    @Suppress("DEPRECATION")
    private fun importBackup() {
        val i = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            // 有的文件管理器把 zip 报成 octet-stream，只写 application/zip 会让备份包
            // 在选择器里变灰选不中——所以放开 MIME，用扩展名和内容自己判。
            type = "*/*"
            putExtra(
                Intent.EXTRA_MIME_TYPES,
                arrayOf("application/zip", "application/x-zip-compressed", "application/octet-stream"),
            )
        }
        runCatching { startActivityForResult(i, REQ_IMPORT) }
            .onFailure { toast("这台设备上没有能打开文件的应用") }
    }

    // 个人词库：导入 / 导出
    //
    // 引擎那边只认文件路径（dict::export_to / import_from），但 SAF 给的是 Uri。
    // 所以这里负责「Uri 与临时文件」这一小段：先落到 cacheDir 里的一个临时文件，
    // 把路径交给引擎，完事就删。为什么不像 [Backup] 那样直接把流递给引擎：
    // core 的依赖政策只有 tyme4rs，不接受「Java 回调的流」这种东西，而为了它
    // 多写一层 shim，比一次临时文件拷贝更没有把握。

    private fun dictExport() {
        val i = Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            // 纯文本：导出来是能用记事本打开、能直接拷到电脑上用的东西，
            // 不是只有本 App 认识的二进制。
            type = "text/plain"
            putExtra(Intent.EXTRA_TITLE, "feihe-dict-" + Backup.stamp() + ".txt")
        }
        runCatching { startActivityForResult(i, REQ_DICT_EXPORT) }
            .onFailure { toast("这台设备上没有能找到的文件管理器") }
    }

    private fun dictImport() {
        val i = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            // 同样放开 MIME：txt 常被报成 octet-stream，写死 text/plain 会变灰
            type = "*/*"
            putExtra(Intent.EXTRA_MIME_TYPES, arrayOf("text/plain", "application/octet-stream"))
        }
        runCatching { startActivityForResult(i, REQ_DICT_IMPORT) }
            .onFailure { toast("这台设备上没有能打开文件的应用") }
    }

    /**
     * 从搜狗词库文件导入：`.scel` 细胞词库 / `.bin` / `.txt`。
     *
     * 和 [dictImport] 是两条路：那边只收「词 + Tab + 全拼」的文本，这边收的是
     * 搜狗自己的格式（词是二进制里抠出来的，拼音要现场注）。用户从搜狗词库网站
     * 下载下来的就是 `.scel`，所以这个入口必须单独存在。
     *
     * MIME 依然放开（不写死类型）：`.scel` 在文件管理器里普遍被报成
     * `application/octet-stream`，写死类型会让文件变灰选不中。
     */
    private fun importSogouDict() {
        val i = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "*/*"
            putExtra(
                Intent.EXTRA_MIME_TYPES,
                arrayOf(
                    "application/octet-stream",
                    "application/x-sogou-scel",
                    "text/plain",
                ),
            )
        }
        runCatching { startActivityForResult(i, REQ_SOGOU_IMPORT) }
            .onFailure { toast("这台设备上没有能打开文件的应用") }
    }

    /**
     * 搜狗词库导入的 Uri → 临时文件 → 引擎。
     *
     * 结果**必须用对话框显示**，不能只弹 Toast：摘要有好几行（读入 N 个文件、
     * 解析 N 条、新增 N 条）。Toast 一长就被截断，用户看到的是半句话。
     * 这也是 2026-11-06 那个「导入后没有任何提示」的一半原因——提示走的是
     * 单行 Toast，且成功路径上没有摘要。
     */
    private fun sogouTransfer(uri: Uri) {
        // 扩展名要保住：引擎按扩展名选解析器（.scel / .bin / .txt），
        // 临时文件叫 dict-in.tmp 的话会被判成「不认识的扩展名」。
        val name = queryDisplayName(uri) ?: "dict.scel"
        val ext = name.substringAfterLast('.', "").lowercase()
        val tmp = File(cacheDir, "sogou-in" + if (ext.isEmpty()) "" else ".$ext")
        toast("导入中…")
        Thread {
            val result = runCatching {
                val input = contentResolver.openInputStream(uri)
                if (input == null) {
                    NativeEngine.SogouImport(false, "打不开这个文件", 0)
                } else {
                    input.use { src -> tmp.outputStream().use { src.copyTo(it) } }
                    NativeEngine.sogouImport(tmp.absolutePath)
                }
            }.getOrElse { NativeEngine.SogouImport(false, it.message ?: "导入失败", 0) }
            tmp.delete()
            runOnUiThread {
                AlertDialog.Builder(this)
                    .setTitle(if (result.ok) "导入完成" else "没能导入")
                    .setMessage(result.message)
                    .setPositiveButton("好", null)
                    .show()
                // 导入成功就要让「当前：…」那行小字和开关状态跟上
                if (result.ok) {
                    cfg = NativeEngine.settings()
                    setContentView(buildUi())
                }
            }
        }.start()
    }

    /** 从 SAF 的 Uri 里问出文件名（带扩展名）。取不到就返回 null。 */
    private fun queryDisplayName(uri: Uri): String? = runCatching {
        contentResolver.query(uri, null, null, null, null)?.use { c ->
            val i = c.getColumnIndex(android.provider.OpenableColumns.DISPLAY_NAME)
            if (i >= 0 && c.moveToFirst()) c.getString(i) else null
        }
    }.getOrNull()

    /** Uri 与临时文件互转，然后交给引擎。返回给用户看的那句话由引擎给。 */
    private fun dictTransfer(uri: Uri, import: Boolean) {
        val tmp = File(cacheDir, if (import) "dict-in.txt" else "dict-out.txt")
        runBusy {
            try {
                if (import) {
                    // 只读、带缓冲：一份词库文本通常在几百 KB 到几 MB。
                    val input = contentResolver.openInputStream(uri)
                    if (input == null) {
                        "ERR 打不开这个文件"
                    } else {
                        input.use { src -> tmp.outputStream().use { src.copyTo(it) } }
                        NativeEngine.dictImport(tmp.absolutePath)
                    }
                } else {
                    val r = NativeEngine.dictExport(tmp.absolutePath)
                    // 引擎写完了才把内容搬到用户选的位置；导出失败就什么都不动，
                    // 免得在用户的下载目录里留下一个空文件。
                    if (r.startsWith("OK")) {
                        val out = contentResolver.openOutputStream(uri)
                        if (out == null) {
                            "ERR 写不进这个位置"
                        } else {
                            out.use { dst -> tmp.inputStream().use { it.copyTo(dst) } }
                            r
                        }
                    } else {
                        r
                    }
                }
            } finally {
                tmp.delete()
            }
        }
    }

    @Deprecated("SAF 的旧回调；没有 androidx.activity 就先用它，行为完全一样")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (resultCode != RESULT_OK) return
        val uri = data?.data ?: return
        when (requestCode) {
            REQ_EXPORT -> runBusy { Backup.export(this, uri) }
            REQ_DICT_EXPORT -> dictTransfer(uri, import = false)
            REQ_DICT_IMPORT -> dictTransfer(uri, import = true)
            REQ_SOGOU_IMPORT -> sogouTransfer(uri)
            REQ_IMPORT -> {
                // 导入是**破坏性**的（先清后写），必须确认一次再动手。
                AlertDialog.Builder(this)
                    .setTitle("导入备份")
                    .setMessage(
                        "会用备份里的数据覆盖当前的设置、个人词库、词频、置顶、自造词、联想。\n\n" +
                            "当前数据不会保留。确定继续吗？",
                    )
                    .setPositiveButton("导入") { _, _ -> runBusy { Backup.import(this, uri) } }
                    .setNegativeButton("取消", null)
                    .show()
            }
        }
    }

    /**
     * 干活 + 汇报结果。
     *
     * **必须离开主线程**：导出要读几 MB，导入完还要重编译个人词库（百毫秒级），
     * 在主线程做就是一次肉眼可见的卡顿。线程里只碰 [Backup] 和引擎，UI 一律回主线程改。
     */
    private fun runBusy(block: () -> String) {
        toast("处理中…")
        Thread {
            val result = runCatching { block() }
            runOnUiThread {
                toast(result.getOrElse { it.message ?: "失败" })
                if (result.isSuccess) {
                    // 导入会换掉 config.ini，方案/输入方式可能变了，整页重建一次
                    cfg = NativeEngine.settings()
                    setContentView(buildUi())
                }
            }
        }.start()
    }

    private fun toast(text: String) {
        Toast.makeText(this, text, Toast.LENGTH_LONG).show()
    }

    /**
     * 语音模型状态 + 隐私说明。
     *
     * 隐私这条必须写在设置页里而不是只在 README：用户有权在授权麦克风之前就知道
     * "录音不出本机"。模型是离线 paraformer（见 `docs/Android-方案.md` §4），
     * 没有任何联网上传路径。
     */
    private fun requestMicPermissionIfNeeded() {
        if (checkSelfPermission(android.Manifest.permission.RECORD_AUDIO) ==
            android.content.pm.PackageManager.PERMISSION_GRANTED
        ) return
        // 只在用户主动打开语音时申请一次；拒绝了也不反复弹，设置页有说明。
        requestPermissions(arrayOf(android.Manifest.permission.RECORD_AUDIO), 1)
    }

    private fun showVoiceInfo() {
        val model = assets.list("asr")?.joinToString(", ") ?: "（缺失）"
        AlertDialog.Builder(this)
            .setTitle("语音模型")
            .setMessage(
                "内置模型文件：$model\n\n" +
                    "识别全程在本机离线完成，录音不会上传、不会保存，也不经过任何网络请求。" +
                    "关闭「语音输入」后本输入法不再申请录音权限。\n\n" +
                    "当前版本只在句末补一个句号，中间标点不猜（猜错比不猜更烦人）。"
            )
            .setPositiveButton("知道了", null)
            .show()
    }

    private fun textBlock(label: String, desc: String): View {
        val box = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        box.addView(
            TextView(this).apply {
                text = label
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 15f)
                setTextColor(TEXT)
            },
        )
        box.addView(
            TextView(this).apply {
                text = desc
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 12f)
                setTextColor(SUB)
            },
        )
        return box
    }

    /** 改一项设置。写失败（引擎没加载）时静默——页面上开关本来也是禁用的。 */
    private fun set(key: String, value: Boolean) = set(key, if (value) "1" else "0")

    private fun set(key: String, value: String) {
        NativeEngine.setSetting(key, value)
    }

    private fun dp(v: Float): Int = (v * resources.displayMetrics.density + 0.5f).toInt()

    /**
     * 配色。**从 [Palette] 取，不在本文件写字面量**——深浅两套由系统夜间模式决定，
     * 页面每次进入都会重建（onCreate -> buildUi），所以退到后台改了系统主题，
     * 回来时颜色是新的，不需要额外监听 onConfigurationChanged。
     */
    private val palette by lazy { Palette.of(this) }
    private val BG get() = palette.bg
    private val CARD get() = palette.card
    private val TEXT get() = palette.text
    private val SUB get() = palette.sub
    private val ACCENT get() = palette.accent
    private val DANGER get() = palette.danger

    private companion object {
        /** 文件选择器的请求码。两个值够用就行，别和别的 startActivityForResult 撞。 */
        const val REQ_EXPORT = 1001
        const val REQ_IMPORT = 1002
        const val REQ_DICT_EXPORT = 1003
        const val REQ_DICT_IMPORT = 1004
        const val REQ_SOGOU_IMPORT = 1005
    }
}
