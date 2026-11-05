package com.feihe.ime

import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.graphics.drawable.GradientDrawable
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
                setTextColor(if (cfg == null) 0xFFC5221F.toInt() else SUB)
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

    @Deprecated("SAF 的旧回调；没有 androidx.activity 就先用它，行为完全一样")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (resultCode != RESULT_OK) return
        val uri = data?.data ?: return
        when (requestCode) {
            REQ_EXPORT -> runBusy { Backup.export(this, uri) }
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

    private companion object {
        val BG = 0xFFF2F3F5.toInt()
        val CARD = 0xFFFFFFFF.toInt()
        val TEXT = 0xFF202124.toInt()
        val SUB = 0xFF5F6368.toInt()
        val ACCENT = 0xFF1A6FD4.toInt()

        /** 文件选择器的请求码。两个值够用就行，别和别的 startActivityForResult 撞。 */
        const val REQ_EXPORT = 1001
        const val REQ_IMPORT = 1002
    }
}
