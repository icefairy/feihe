package com.feihe.ime.voice

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.IBinder
import android.util.Log
import java.util.concurrent.atomic.AtomicBoolean

/**
 * 在通知栏里下载语音模型。
 *
 * ## 为什么必须是前台服务
 *
 * 81 MB 在国内网络下要一分钟上下。普通后台线程会被系统在**应用切到后台时**掐掉
 * （尤其是国产 ROM 的省电策略），用户点完下载、切去微信回个消息，回来发现下到一半没了。
 * 前台服务 + 常驻通知能挡住这个——通知本身就是「我在干活」的声明。
 *
 * 顺带满足了用户的另一个要求：**通知栏能看进度**。这不需要额外做什么，
 * 前台服务的通知本来就该带进度条。
 *
 * ## 为什么用 Service 而不是 WorkManager
 *
 * 不引新依赖（`implementation("androidx.work:work-runtime")` 加进来 APK 大 1 MB+），
 * 而且这个任务是一次性的、用户主动触发的、需要实时进度——WorkManager 的调度延迟
 * 和「进度只能靠 setProgress」反而更绕。
 *
 * ## 停止的语义
 *
 * 用户从通知栏点「取消」→ [ACTION_CANCEL]。已经下好的部分**不删**（`.part` 留着），
 * 下次点下载从断点续传。
 */
class ModelDownloadService : Service() {

    companion object {
        private const val TAG = "FeiheModelDl"
        private const val CHANNEL_ID = "feihe_model_download"
        private const val NOTIF_ID = 0x4D4C // "ML"

        const val ACTION_START = "com.feihe.ime.MODEL_DOWNLOAD_START"
        const val ACTION_CANCEL = "com.feihe.ime.MODEL_DOWNLOAD_CANCEL"

        /** 下载状态广播（只给本应用内部用，SettingsActivity 订阅着刷新 UI）。 */
        const val ACTION_PROGRESS = "com.feihe.ime.MODEL_DOWNLOAD_PROGRESS"
        const val EXTRA_DONE = "done"
        const val EXTRA_TOTAL = "total"
        const val EXTRA_NAME = "name"
        const val EXTRA_STATE = "state"

        const val STATE_RUNNING = "running"
        const val STATE_OK = "ok"
        const val STATE_FAIL = "fail"
        const val STATE_CANCELLED = "cancelled"

        /** 有下载在跑。设置页据此决定按钮显示「下载」还是「下载中…」。 */
        @Volatile
        var running: Boolean = false
            private set

        fun start(ctx: Context) {
            val i = Intent(ctx, ModelDownloadService::class.java).setAction(ACTION_START)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                ctx.startForegroundService(i)
            } else {
                ctx.startService(i)
            }
        }

        fun cancel(ctx: Context) {
            ctx.startService(Intent(ctx, ModelDownloadService::class.java).setAction(ACTION_CANCEL))
        }
    }

    private val cancelled = AtomicBoolean(false)
    private var thread: Thread? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_CANCEL -> {
                cancelled.set(true)
                return START_NOT_STICKY
            }
            ACTION_START -> {
                if (running) return START_NOT_STICKY
                // **进前台失败就必须放弃下载。**
                //
                // 系统对 `startForegroundService()` 有 5 秒倒计时：到点没调到
                // `startForeground()` 就抛 `ForegroundServiceDidNotStartInTimeException`
                // ——那是进程级 FATAL，整个输入法直接闪退，用户正在写的东西全没。
                // 以前这里吞掉了异常继续下载，正好踩中这条。
                if (!startForegroundSafely()) {
                    running = false
                    stopSelf()
                    return START_NOT_STICKY
                }
                begin()
                return START_NOT_STICKY
            }
        }
        return START_NOT_STICKY
    }

    /**
     * 进前台。
     *
     * Android 12+ 禁止从后台启动前台服务（`ForegroundServiceStartNotAllowedException`），
     * 但这里是用户点了按钮触发的，属于允许的场景；仍然 try/catch 兜一层——
     * 抛出来会直接崩输入法进程，而下载没开始就被杀掉，用户只看到闪退。
     */
    private fun startForegroundSafely(): Boolean {
        return try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                val nm = getSystemService(NotificationManager::class.java)
                if (nm.getNotificationChannel(CHANNEL_ID) == null) {
                    nm.createNotificationChannel(
                        NotificationChannel(
                            CHANNEL_ID,
                            "语音模型下载",
                            // LOW：不出声、不弹横幅。用户正在打字，进度条自己会动，
                            // 不需要再响一声打断他。
                            NotificationManager.IMPORTANCE_LOW,
                        ).apply { setShowBadge(false) },
                    )
                }
            }
            startForeground(NOTIF_ID, buildNotification(0, ModelDownloader.totalBytes, "准备下载…"))
            true
        } catch (e: Throwable) {
            // 常见原因：漏声明 FOREGROUND_SERVICE 权限、Android 12+ 从后台启动、
            // 通知权限被拒。都属「这台设备/这个时机下不可用」，不该让进程陪葬。
            Log.e(TAG, "前台服务启动失败：${e.message}")
            sendBroadcast(
                Intent(ACTION_PROGRESS)
                    .setPackage(packageName)
                    .putExtra(EXTRA_STATE, STATE_FAIL)
                    .putExtra(EXTRA_NAME, "无法启动下载服务：${e.message}"),
            )
            false
        }
    }

    private fun buildNotification(done: Long, total: Long, text: String): Notification {
        val builder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Notification.Builder(this, CHANNEL_ID)
        } else {
            @Suppress("DEPRECATION")
            Notification.Builder(this)
        }
        val pct = if (total > 0) (done * 100 / total).toInt() else 0
        builder
            .setContentTitle("语音模型下载")
            .setContentText(if (total > 0) "$text  $pct%" else text)
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setProgress(if (total > 0) 100 else 0, pct, total <= 0)
        // 取消按钮。Android 13+ 会忽略 addAction 的图标，但按钮照常显示
        builder.addAction(
            Notification.Action.Builder(
                null,
                "取消",
                android.app.PendingIntent.getService(
                    this,
                    1,
                    Intent(this, ModelDownloadService::class.java).setAction(ACTION_CANCEL),
                    android.app.PendingIntent.FLAG_UPDATE_CURRENT or
                        (if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M)
                            android.app.PendingIntent.FLAG_IMMUTABLE else 0),
                ),
            ).build(),
        )
        return builder.build()
    }

    private fun notifyProgress(done: Long, total: Long, name: String) {
        val nm = getSystemService(NotificationManager::class.java) ?: return
        runCatching { nm.notify(NOTIF_ID, buildNotification(done, total, name)) }
        // 广播给设置页：它可能正开着，进度条要跟着动
        sendBroadcast(
            Intent(ACTION_PROGRESS)
                .setPackage(packageName)
                .putExtra(EXTRA_DONE, done)
                .putExtra(EXTRA_TOTAL, total)
                .putExtra(EXTRA_NAME, name)
                .putExtra(EXTRA_STATE, STATE_RUNNING),
        )
    }

    private fun begin() {
        running = true
        thread = Thread {
            val err = ModelDownloader.downloadAll(
                this,
                onProgress = { done, total, name -> notifyProgress(done, total, name) },
                isCancelled = { cancelled.get() },
            )
            running = false
            val state = when {
                cancelled.get() -> STATE_CANCELLED
                err == null -> STATE_OK
                else -> STATE_FAIL
            }
            sendBroadcast(
                Intent(ACTION_PROGRESS)
                    .setPackage(packageName)
                    .putExtra(EXTRA_STATE, state)
                    .putExtra(EXTRA_NAME, err ?: "")
                    .putExtra(EXTRA_DONE, ModelDownloader.presentFiles(this).sumOf { it.bytes })
                    .putExtra(EXTRA_TOTAL, ModelDownloader.totalBytes),
            )
            if (err != null) Log.w(TAG, "下载失败：$err")
            // 成功时留一条「已完成」再退，用户能在通知栏确认结果；
            // 失败也留一条，不然点完没反应像坏了
            val nm = getSystemService(NotificationManager::class.java)
            if (state == STATE_OK) {
                runCatching {
                    nm?.notify(
                        NOTIF_ID,
                        buildNotification(ModelDownloader.totalBytes, ModelDownloader.totalBytes, "已完成")
                            .let {
                                // 完成态去掉取消按钮和 ongoing，让用户能划掉
                                Notification.Builder(this, CHANNEL_ID)
                                    .setContentTitle("语音模型已就绪")
                                    .setContentText("可以开始语音输入了")
                                    .setSmallIcon(android.R.drawable.stat_sys_download_done)
                                    .setAutoCancel(true)
                                    .build()
                            },
                    )
                }
            }
            stopForeground(
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
                    STOP_FOREGROUND_DETACH
                } else {
                    0
                },
            )
            stopSelf()
        }.also { it.isDaemon = true }
        thread?.start()
    }

    override fun onDestroy() {
        cancelled.set(true)
        running = false
        super.onDestroy()
    }
}
