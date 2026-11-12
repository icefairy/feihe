package com.feihe.ime.voice

import android.content.Context
import android.util.Log
import java.io.File
import java.io.RandomAccessFile
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest

/**
 * 语音模型的下载源 + 落盘规则。
 *
 * ## 为什么从魔搭（ModelScope）下
 *
 * 内置模型 81 MB，打进 APK 就是所有人替「不用语音」的用户背这 81 MB。改成按需下载之后
 * 需要一个**国内能直连**的地址：
 *
 * - `huggingface.co` / `hf-mirror.com`：前者国内不通，后者是第三方反代，随时可能失效
 * - `github.com`：这台构建机实测完全不通（http=000）
 * - **ModelScope `pengzhendong/sherpa-onnx-paraformer-zh-small`**：实测直连
 *   1.55 MB/s、支持 Range（HTTP 206），81 MB 约 52 秒
 *
 * 文件大小和 sha256 与原来 `tools/fetch_asr_model.py` 钉的值**逐字节一致**
 * （81828675 / 3ef6c19…），所以「内置」和「下载」得到的是同一份模型。
 *
 * ## 为什么下到 filesDir 而不是 assets
 *
 * assets 是只读的、编译期就定了，运行期没办法往里写。sherpa-onnx 的
 * `OfflineRecognizer(assetManager, config)` 和 `(filePath, config)` 是两套构造，
 * 下载来的模型走后者（见 [VoiceEngine]）。
 */
object ModelDownloader {

    private const val TAG = "FeiheModelDl"

    /** 仓库 + 分支。换源只改这两个常量。 */
    private const val REPO = "pengzhendong/sherpa-onnx-paraformer-zh-small"
    private const val REVISION = "master"
    private const val BASE = "https://modelscope.cn/models/$REPO/resolve/$REVISION"

    /**
     * 每个文件：远端名 → (期望字节数, sha256)。
     *
     * 这两个值不是抄来的，是 2026-11-05 实拉后钉住的（和 `tools/fetch_asr_model.py`
     * 的 `EXPECTED` 同一份）。**校验必须做**：80 MB 传到一半断掉、运营商劫持插广告页、
     * 上游偷偷换模型，这三种情况在「文件下完了」这个判断上看起来都正常，
     * 只有哈希能区分。
     */
    val FILES: List<ModelFile> = listOf(
        ModelFile(
            remote = "model.int8.onnx",
            bytes = 81_828_675L,
            sha256 = "3ef6c19369b912f7caf3cef8e545c5ccd1a33d9d7ec792a46668dc41c4b229ec",
        ),
        ModelFile(
            remote = "tokens.txt",
            bytes = 75_352L,
            sha256 = "4b2d964e18b9cf139b473003b6698fb2ed9a2a5ec55b93daa677b28f578897aa",
        ),
    )

    /** 模型目录。和内置 assets 的 `asr/` 同名同级，方便两套加载路径共用文件名。 */
    fun dir(ctx: Context): File = File(ctx.filesDir, "asr").apply { mkdirs() }

    fun target(ctx: Context, remote: String): File = File(dir(ctx), remote)

    fun url(remote: String): String = "$BASE/$remote"

    /** 总字节数（下载进度按它算百分比）。 */
    val totalBytes: Long get() = FILES.sumOf { it.bytes }

    /**
     * 已完整下好的文件。**逐个校验大小**，不校验哈希（哈希在下载结束时验一次就够，
     * 每次诊断都算 81 MB 的 sha256 要两秒）。
     */
    fun presentFiles(ctx: Context): List<ModelFile> =
        FILES.filter { f ->
            val p = target(ctx, f.remote)
            p.isFile && p.length() == f.bytes
        }

    fun isComplete(ctx: Context): Boolean = presentFiles(ctx).size == FILES.size

    /** 缺哪些文件的可读描述，设置页/对话框显示用。 */
    fun missingNames(ctx: Context): List<String> =
        FILES.filterNot { f -> target(ctx, f.remote).isFile }.map { it.remote }

    data class ModelFile(val remote: String, val bytes: Long, val sha256: String)

    /** 下载中的一个文件的进度回调。 */
    fun interface Progress {
        /** @param done 全任务已完成字节（含之前文件），@param total 全任务总字节 */
        fun onProgress(done: Long, total: Long, currentName: String)
    }

    /**
     * 下载全部缺失文件（**必须在后台线程调**）。
     *
     * 已存在且大小正确的文件跳过——所以「下到 60% 断了」再点一次会从断点续传，
     * 而不是重头来。断点靠 `.part` 文件 + HTTP Range 实现。
     *
     * @return null = 成功；非 null = 失败原因（给 UI 显示，不是给日志）
     */
    fun downloadAll(
        ctx: Context,
        onProgress: Progress,
        isCancelled: () -> Boolean = { false },
    ): String? {
        val doneBefore = presentFiles(ctx).sumOf { it.bytes }
        var done = doneBefore
        onProgress.onProgress(done, totalBytes, "")

        for (f in FILES) {
            if (isCancelled()) return "已取消"
            val dest = target(ctx, f.remote)
            if (dest.isFile && dest.length() == f.bytes) continue
            val err = downloadOne(f, dest) { got ->
                onProgress.onProgress(doneBefore.coerceAtMost(done) + got, totalBytes, f.remote)
            }
            if (err != null) return err
            // 下载完成立刻校验哈希：对不上说明文件是坏的/被换过，删掉让用户重试
            val actual = sha256Of(dest)
            if (actual != f.sha256) {
                dest.delete()
                return "${f.remote} 校验失败（sha256 $actual），已删除，请重试"
            }
            done = presentFiles(ctx).sumOf { it.bytes }
        }
        return null
    }

    /**
     * 下单个文件，带 Range 续传。
     *
     * @param onBytes 本文件已下字节数（不含前一个文件）
     */
    private fun downloadOne(
        f: ModelFile,
        dest: File,
        onBytes: (Long) -> Unit,
    ): String? {
        val part = File(dest.parentFile, dest.name + ".part")
        var existing = if (part.isFile) part.length() else 0L
        // `.part` 已经比目标还大 → 是上次残留的垃圾，重来
        if (existing > f.bytes) {
            part.delete()
            existing = 0
        }
        if (existing == f.bytes) {
            // 上次其实下完了、只差改名
            return if (part.renameTo(dest)) null else "改名失败"
        }
        val conn = try {
            (URL(url(f.remote)).openConnection() as HttpURLConnection).apply {
                connectTimeout = 20_000
                readTimeout = 20_000
                instanceFollowRedirects = true
                setRequestProperty("User-Agent", "feihe-ime/1")
                if (existing > 0) setRequestProperty("Range", "bytes=$existing-")
            }
        } catch (e: Exception) {
            return "连接失败：${e.message}"
        }

        try {
            val code = conn.responseCode
            // 206 = 服务器接受续传；200 = 服务器不支持 Range，从头来
            if (code != 200 && code != 206) return "HTTP $code"
            if (code == 200) existing = 0
            val bodyOffset = existing

            RandomAccessFile(part, "rw").use { raf ->
                raf.seek(bodyOffset)
                conn.inputStream.use { ins ->
                    val buf = ByteArray(64 * 1024)
                    var got = 0L
                    var lastReport = 0L
                    while (true) {
                        val n = ins.read(buf)
                        if (n <= 0) break
                        raf.write(buf, 0, n)
                        got += n
                        // 每 256 KB 报一次：再密就是对 UI 线程的骚扰，
                        // 再疏进度条看着像卡死
                        if (got - lastReport >= 256 * 1024) {
                            lastReport = got
                            onBytes(bodyOffset + got)
                        }
                    }
                    onBytes(bodyOffset + got)
                }
            }
            if (part.length() != f.bytes) {
                return "${f.remote} 大小不符：期望 ${f.bytes}，实得 ${part.length()}"
            }
            if (!part.renameTo(dest)) return "改名失败：${f.remote}"
            Log.i(TAG, "下载完成 ${f.remote}")
            return null
        } catch (e: Exception) {
            // 断了不算失败——`.part` 留着，下次续传
            Log.w(TAG, "下载中断 ${f.remote}：${e.message}")
            return "下载中断：${e.message}（已保留进度，可重试续传）"
        } finally {
            conn.disconnect()
        }
    }

    fun sha256Of(f: File): String {
        val md = MessageDigest.getInstance("SHA-256")
        f.inputStream().use { ins ->
            val buf = ByteArray(1 shl 20)
            while (true) {
                val n = ins.read(buf)
                if (n <= 0) break
                md.update(buf, 0, n)
            }
        }
        return md.digest().joinToString("") { "%02x".format(it) }
    }

    /** 已有但大小不对的残缺文件清单（`.part` 不含在内），诊断用。 */
    fun brokenFiles(ctx: Context): List<String> =
        FILES.filter { f ->
            val p = target(ctx, f.remote)
            p.isFile && p.length() != f.bytes
        }.map { it.remote }
}
