package com.feihe.ime

import android.app.Activity
import android.content.Intent
import android.graphics.drawable.GradientDrawable
import android.os.Bundle
import android.provider.Settings
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.LinearLayout
import android.widget.TextView

/**
 * 首次启用引导页。
 *
 * ## 为什么需要这一页
 *
 * 输入法和普通 App 的安装体验完全不同：装完**不会**出现在桌面上能用，也不会
 * 自动生效。用户必须做两件事，而且这两件事一个在「设置 → 语言和输入法 → 键盘」
 * 里、一个在「当前输入法」里，很多人在系统设置里绕了十几层还没找到。
 *
 * 别的输入法（豆包/搜狗/讯飞）都有一页两步引导，我们以前完全没有——
 * 用户装完只能靠猜。这一页就是补那个洞。
 *
 * ## 为什么状态每次现算，不记「已完成到第几步」
 *
 * 见 [SetupState] 的注释：用户可能做完第一步没做第二步，也可能做完两步之后
 * 又把输入法换回去了。按流程记忆来渲染，三种真实状态里会显示错两种。
 *
 * 所以每次 [onResume] 都重新读系统设置、重新渲染。用户跳去系统设置点完再回来，
 * 页面自动变成下一步——**不需要他手动点「我已完成」**。
 *
 * ## 两步分别跳哪里
 *
 * - **第一步 启用**：直接跳系统的输入法启用页（Android 11+ 用
 *   [Settings.ACTION_INPUT_METHOD_SETTINGS]，更老的系统没有这个 action）。
 * - **第二步 切换**：弹系统输入法选择器 [InputMethodManager.showInputMethodPicker]，
 *   这是唯一能「换当前输入法」的公开接口，不能直接跳某个设置页。
 */
class SetupActivity : Activity() {

    private val palette by lazy { Palette.of(this) }
    private val BG get() = palette.bg
    private val CARD get() = palette.card
    private val TEXT get() = palette.text
    private val SUB get() = palette.sub
    private val ACCENT get() = palette.accent
    private val ON_ACCENT get() = palette.onAccent

    /** 当前状态，[refresh] 每次重算后给 UI 用。 */
    private var state: SetupState = SetupState(enabled = false, selected = false)

    private var root: LinearLayout? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(buildRoot())
        refresh()
    }

    /**
     * 每次回到这一页都重算状态。
     *
     * 用户跳去系统设置勾完再回来，这里必须立刻反映出来——否则他看到还是
     * 「第一步未完成」，会以为勾选没成功，又回去勾一遍。
     */
    override fun onResume() {
        super.onResume()
        refresh()
    }

    /** 读系统设置里的两个真实值，重画。 */
    private fun refresh() {
        state = SetupState.fromSystem(
            enabledIds = readSetting(Settings.Secure.ENABLED_INPUT_METHODS),
            defaultId = readSetting(Settings.Secure.DEFAULT_INPUT_METHOD),
            packageName = packageName,
        )
        rebuild()
    }

    /**
     * 读 `Settings.Secure` 里的字符串。
     *
     * 少数定制系统会把这两项藏起来（返回 null）。**不要因此崩**，也不要
     * 显示「未知」——按「未完成」处理，用户照着点一遍就是了，最坏是白点一次。
     */
    private fun readSetting(key: String): String? =
        runCatching { Settings.Secure.getString(contentResolver, key) }.getOrNull()

    private fun rebuild() {
        root?.removeAllViews()
        val r = root ?: return

        r.addView(TextView(this).apply {
            text = "飞鹤输入法"
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 26f)
            setTextColor(TEXT)
            gravity = Gravity.CENTER
            setPadding(0, dp(48f), 0, dp(8f))
        })

        r.addView(TextView(this).apply {
            text = "只差两步就能开始打字"
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 15f)
            setTextColor(SUB)
            gravity = Gravity.CENTER
            setPadding(0, 0, 0, dp(36f))
        })

        r.addView(stepCard(
            number = "第一步",
            title = "启用飞鹤输入法",
            desc = "在系统设置里勾选「飞鹤输入法」",
            done = state.stepEnabledDone,
            primary = !state.stepEnabledDone,
        ) {
            openEnableSettings()
        })

        r.addView(stepCard(
            number = "第二步",
            title = "切换到飞鹤输入法",
            desc = "在弹出列表里选「飞鹤输入法」",
            done = state.stepSelectedDone,
            primary = state.stepEnabledDone && !state.stepSelectedDone,
        ) {
            showImePicker()
        })

        // 两步都完成：给一条收敛提示 + 直接关掉。
        // **不要自动退出**——用户可能是从设置里点进来的，想确认一下状态。
        if (state.allDone) {
            r.addView(TextView(this).apply {
                text = "✓ 已经可以开始打字了"
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 15f)
                setTextColor(ACCENT)
                gravity = Gravity.CENTER
                setPadding(0, dp(28f), 0, 0)
            })
            r.addView(button("完成", filled = true) { finish() }.apply {
                (layoutParams as? LinearLayout.LayoutParams)?.topMargin = dp(20f)
            })
        }

        // **设置入口**。桌面图标现在指向这一页，如果这里不给入口，
        // 用户想改双拼方案/皮肤就再也进不去设置了（系统设置那条路太深，
        // 键盘状态条的齿轮要先能打字）。
        r.addView(TextView(this).apply {
            text = "进入设置"
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 14f)
            setTextColor(SUB)
            gravity = Gravity.CENTER
            setPadding(0, dp(28f), 0, dp(12f))
            setOnClickListener {
                runCatching {
                    startActivity(Intent(this@SetupActivity, SettingsActivity::class.java))
                }.onFailure { toast("打不开设置页") }
            }
        }.apply {
            (layoutParams as? LinearLayout.LayoutParams)?.topMargin = dp(10f)
        })

        r.addView(TextView(this).apply {
            text = "随时可以在「设置 → 语言和输入法」里改回来"
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 12f)
            setTextColor(SUB)
            gravity = Gravity.CENTER
            setPadding(0, dp(8f), 0, 0)
        })
    }

    /**
     * 一张步骤卡：编号 + 标题 + 说明 + 状态。
     *
     * 已完成的步骤**不再显示按钮**，改成一行绿字「已完成」——留着按钮会让用户
     * 以为自己还没做完，反复点。未完成但还没轮到的步骤（第二步在第一步没做时）
     * 保持灰色但可点：有些人会想先切换，系统自己会引导他去启用。
     */
    private fun stepCard(
        number: String,
        title: String,
        desc: String,
        done: Boolean,
        primary: Boolean,
        onClick: () -> Unit,
    ): View {
        val card = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            background = GradientDrawable().apply {
                cornerRadius = dp(16f).toFloat()
                setColor(CARD)
            }
            setPadding(dp(20f), dp(18f), dp(20f), dp(18f))
            layoutParams = LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
            ).apply { bottomMargin = dp(14f) }
            if (!done) setOnClickListener { onClick() }
        }

        card.addView(TextView(this).apply {
            text = if (done) "$number · 已完成" else number
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 12f)
            setTextColor(if (done) ACCENT else SUB)
        })

        card.addView(TextView(this).apply {
            text = title
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 17f)
            setTextColor(TEXT)
            setPadding(0, dp(6f), 0, dp(4f))
        })

        card.addView(TextView(this).apply {
            text = if (done) "✓ 不需要再操作" else desc
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 13f)
            setTextColor(if (done) ACCENT else SUB)
        })

        if (!done) {
            // 蓝色高亮只给「当前该做的那一步」。两步都灰或都蓝，用户不知道该先点哪个。
            card.addView(TextView(this).apply {
                text = if (primary) "去设置  ›" else "去设置  ›"
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 15f)
                setTextColor(if (primary) ACCENT else SUB)
                setPadding(0, dp(12f), 0, 0)
                gravity = Gravity.END
            })
        }
        return card
    }

    /** 兜底用的整宽按钮（只在全部完成时出现）。 */
    private fun button(text: String, filled: Boolean, onClick: () -> Unit): View =
        TextView(this).apply {
            this.text = text
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 16f)
            gravity = Gravity.CENTER
            setPadding(0, dp(14f), 0, dp(14f))
            setTextColor(if (filled) ON_ACCENT else TEXT)
            background = GradientDrawable().apply {
                cornerRadius = dp(12f).toFloat()
                setColor(if (filled) ACCENT else CARD)
            }
            setOnClickListener { onClick() }
        }

    /**
     * 打开系统「输入法启用」页。
     *
     * Android 11(30) 起才有 `ACTION_INPUT_METHOD_SETTINGS` 这个公开 action。
     * 更老的系统上退到「语言和输入法」总页——那一页里也能找到开关，
     * 只是多一层。**不能什么都不做**：用户点了没反应只会以为 App 坏了。
     */
    private fun openEnableSettings() {
        val candidates = buildList {
            if (android.os.Build.VERSION.SDK_INT >= 30) {
                add(Intent(Settings.ACTION_INPUT_METHOD_SETTINGS))
            }
            add(Intent(Settings.ACTION_LOCALE_SETTINGS))
            add(Intent(Settings.ACTION_SETTINGS))
        }
        for (i in candidates) {
            i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            if (runCatching { startActivity(i) }.isSuccess) return
        }
        toast("打不开系统设置，请手动到「设置 → 语言和输入法」里启用")
    }

    /**
     * 弹系统输入法选择器。
     *
     * 这是**唯一**的公开接口能换当前输入法：系统不允许 App 直接改默认输入法
     * （那等于允许任意应用劫持键盘，安全上不可接受）。
     */
    private fun showImePicker() {
        val imm = getSystemService(android.view.inputmethod.InputMethodManager::class.java)
        if (imm == null) {
            toast("系统里找不到输入法服务")
            return
        }
        runCatching { imm.showInputMethodPicker() }
            .onFailure { toast("打不开输入法选择器，请从通知栏或系统设置切换") }
    }

    private fun buildRoot(): View {
        val outer = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(BG)
        }
        // 小屏/横屏时内容可能超出，套一层滚动
        val scroll = android.widget.ScrollView(this).apply {
            isFillViewport = true
            addView(outer, ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
            ))
        }
        root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(24f), 0, dp(24f), dp(32f))
        }
        outer.addView(root, LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.WRAP_CONTENT,
        ))
        return scroll
    }

    private fun toast(msg: String) =
        android.widget.Toast.makeText(this, msg, android.widget.Toast.LENGTH_SHORT).show()

    private fun dp(v: Float): Int =
        (v * resources.displayMetrics.density + 0.5f).toInt()
}
