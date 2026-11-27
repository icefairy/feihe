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
    private const val KEY_QWERTY_DIGITS_ROW = "qwerty_digits_row"
    private const val KEY_VOICE = "voice"
    private const val KEY_SMS_OTP = "sms_otp"
    private const val KEY_VOICE_CORRECT = "voice_correct"
    private const val KEY_DEBUG_BIG_KEYS = "debug_big_keys"

    private fun prefs(ctx: Context) =
        ctx.getSharedPreferences(FILE, Context.MODE_PRIVATE)

    /** 数字选词行是否显示。默认开——A2 要的是「可隐藏」而不是「默认关掉没人发现」。 */
    fun showNumberRow(ctx: Context): Boolean = prefs(ctx).getBoolean(KEY_NUMBER_ROW, true)

    /**
     * 字母页顶排数字行（1-0 直按输入）是否显示。默认开。
     * 与 [showNumberRow]（候选区数字选词行）是**两个不同的东西**：
     * 这个是键盘顶上一排实体数字键，那个是候选条上方按数字选词的行。
     */
    fun showQwertyDigitsRow(ctx: Context): Boolean =
        prefs(ctx).getBoolean(KEY_QWERTY_DIGITS_ROW, true)

    fun setShowQwertyDigitsRow(ctx: Context, on: Boolean) =
        prefs(ctx).edit().putBoolean(KEY_QWERTY_DIGITS_ROW, on).apply()

    fun setShowNumberRow(ctx: Context, on: Boolean) =
        prefs(ctx).edit().putBoolean(KEY_NUMBER_ROW, on).apply()

    /**
     * 语音面板开关。默认开；没模型/没授权时面板自己会降级为不可用。
     */
    fun voiceEnabled(ctx: Context): Boolean = prefs(ctx).getBoolean(KEY_VOICE, true)

    fun setVoiceEnabled(ctx: Context, on: Boolean) =
        prefs(ctx).edit().putBoolean(KEY_VOICE, on).apply()

    /**
     * 语音识别结果 × 自造词库 的本地纠错。**默认关。**
     *
     * 这个功能会**改写用户说的话**——属于「宁可没有也不能错」的那一类。
     * 不主动打开就不该有任何一次识别结果被替换。判据见
     * [com.feihe.ime.voice.VoiceCorrect]：拼音完全吻合才替换、长度必须一致、可撤销。
     */
    fun voiceCorrect(ctx: Context): Boolean = prefs(ctx).getBoolean(KEY_VOICE_CORRECT, false)

    fun setVoiceCorrect(ctx: Context, on: Boolean) =
        prefs(ctx).edit().putBoolean(KEY_VOICE_CORRECT, on).apply()

    /**
     * 从短信读取验证码。**默认关**。
     *
     * 这是整个输入法里唯一会碰「别人发给你的内容」的功能，读的还是短信——权限本身
     * 就是敏感权限。所以默认值必须是 false：用户没主动打开，就不该有任何一条短信
     * 被读。打开后还有两道闸（系统授权 + 当前是验证码框），见 [com.feihe.ime.clip.SmsOtp]。
     */
    fun smsOtp(ctx: Context): Boolean = prefs(ctx).getBoolean(KEY_SMS_OTP, false)

    fun setSmsOtp(ctx: Context, on: Boolean) =
        prefs(ctx).edit().putBoolean(KEY_SMS_OTP, on).apply()

    /**
     * **调试用大键盘**。默认关。
     *
     * 打开后键高从 46dp 放大到 120dp、字号同比放大，用来排查「点击落错键」
     * 「命中区与视觉键帽不一致」这类纯几何问题——正常尺寸下偏差被压缩，肉眼看不出来。
     *
     * 为什么做成运行时开关而不是编译期分支：切布局要重新打包的那几轮，光打包就
     * 浪费十几分钟，还容易把调试分支忘在正式包里。开关是同一个包内即时生效。
     */
    fun debugBigKeys(ctx: Context): Boolean = prefs(ctx).getBoolean(KEY_DEBUG_BIG_KEYS, false)

    fun setDebugBigKeys(ctx: Context, on: Boolean) =
        prefs(ctx).edit().putBoolean(KEY_DEBUG_BIG_KEYS, on).apply()

    /** 调试放大倍数：正常 1f，大键盘 2.6f（46dp → 120dp）。 */
    fun keyScale(ctx: Context): Float = if (debugBigKeys(ctx)) 2.6f else 1f
}

