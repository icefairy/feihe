package com.feihe.ime

import android.app.Activity
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
                isEnabled = cfg != null
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
    }
}
