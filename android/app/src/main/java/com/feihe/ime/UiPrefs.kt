package com.feihe.ime

import android.content.Context

/**
 * 纯 Android 侧的界面开关。
 *
 * ## 为什么不进 `config.ini`
 *
 * 引擎配置（`core::config`）是 **Windows 和 Android 共用**的：方案、输入方式、繁体、
 * 联想、学习。往里加一个 `number_row` 字段，桌面端就要跟着解析一个自己永远用不到、
 * 也永远不显示的键。而这个开关描述的只是"手机上候选条下面要不要多一排按钮"，
 * 是壳的呈现方式，不是引擎的行为。
 *
 * 所以边界划在这里：**引擎决定"能打出什么"，壳决定"怎么摆给人看"**。
 * 代价是这些开关不会随 `config.ini` 备份走——可接受，它们不是用户数据，
 * 换台机器重新勾一次即可。
 */
object UiPrefs {
    private const val FILE = "feihe_ui"
    private const val KEY_NUMBER_ROW = "number_row"
    private const val KEY_VOICE = "voice"

    private fun prefs(ctx: Context) =
        ctx.getSharedPreferences(FILE, Context.MODE_PRIVATE)

    /** 数字选词行是否显示。默认开——A2 要的是「可隐藏」而不是「默认关掉没人发现」。 */
    fun showNumberRow(ctx: Context): Boolean = prefs(ctx).getBoolean(KEY_NUMBER_ROW, true)

    fun setShowNumberRow(ctx: Context, on: Boolean) =
        prefs(ctx).edit().putBoolean(KEY_NUMBER_ROW, on).apply()

    /** 语音面板开关。默认开；没模型/没授权时面板自己会降级为不可用。 */
    fun voiceEnabled(ctx: Context): Boolean = prefs(ctx).getBoolean(KEY_VOICE, true)

    fun setVoiceEnabled(ctx: Context, on: Boolean) =
        prefs(ctx).edit().putBoolean(KEY_VOICE, on).apply()
}
