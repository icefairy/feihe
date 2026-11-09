package com.feihe.ime.clip

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.net.Uri

/**
 * 从短信里取验证码。
 *
 * ## 默认关闭，而且只在「验证码框」里碰
 *
 * 读短信是**敏感权限**（`READ_SMS`），它比剪贴板更私有：剪贴板里是用户自己复制的东西，
 * 短信是别人发来的。所以这里有三道闸，缺一个都不读：
 *
 * 1. 用户在设置里显式打开「从短信读取验证码」（[com.feihe.ime.UiPrefs.smsOtp]，默认关）；
 * 2. 系统里真的给了 `READ_SMS` 权限（[canRead]）；
 * 3. 当前输入框**像验证码框**（[OtpCode.looksLikeOtpField]）——平时打字一个字都不读。
 *
 * 只取**最近 10 分钟**的十几条里的最新几条，取到验证码就把整条短信丢掉：
 * 正文不落盘、不进剪贴板历史、不打日志。[latestCode] 返回的就是那 4-8 位数字本身。
 */
object SmsOtp {

    /**
     * 只看最近 10 分钟的短信。
     *
     * 短信验证码的有效期普遍是 5-10 分钟，更早的码即使还在收件箱里也已经失效；
     * 把范围压小，既是隐私（少读），也是准确（不会提示一个过期码）。
     */
    const val WINDOW_MS = 10L * 60 * 1000

    /** 最多看几条。短信 App 的推广短信可能夹在中间，给两条冗余就够，不必翻整个收件箱。 */
    private const val LOOK_AT = 3

    private val INBOX: Uri = Uri.parse("content://sms/inbox")

    /** 有没有 `READ_SMS` 权限。没有就什么都不做，不弹框、不提示——设置页会说明。 */
    fun canRead(ctx: Context): Boolean =
        ctx.checkSelfPermission(Manifest.permission.READ_SMS) == PackageManager.PERMISSION_GRANTED

    /**
     * 最近窗口内最新的一条能解析出验证码的短信，没有就返回 null。
     *
     * 取不到（没权限 / 收件箱是空的 / 厂商改过 provider）一律返回 null，**不抛异常**：
     * 输入法不能因为读短信失败就影响打字。
     */
    fun latestCode(ctx: Context, now: Long): String? {
        if (!canRead(ctx)) return null
        val cursor = runCatching {
            ctx.contentResolver.query(
                INBOX,
                arrayOf("body", "date"),
                "date > ?",
                arrayOf((now - WINDOW_MS).toString()),
                "date DESC",
            )
        }.getOrNull() ?: return null

        cursor.use { c ->
            var i = 0
            while (i < LOOK_AT && c.moveToNext()) {
                val body = runCatching { c.getString(0) }.getOrNull()
                val code = OtpCode.extract(body)
                if (code != null) return code
                i++
            }
        }
        return null
    }
}
