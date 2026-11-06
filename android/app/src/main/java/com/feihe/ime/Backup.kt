package com.feihe.ime

import android.content.Context
import android.net.Uri
import android.util.Log
import com.feihe.ime.engine.NativeEngine
import java.io.BufferedInputStream
import java.io.BufferedOutputStream
import java.io.File
import java.io.OutputStream
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.zip.ZipEntry
import java.util.zip.ZipInputStream
import java.util.zip.ZipOutputStream

/**
 * 用户数据的导出 / 导入。
 *
 * ## 谁管什么
 *
 * - **引擎**（`feihe_core::backup`）回答「哪些文件算用户数据」和「导入后怎么让它
 *   生效」。清单只有一份，引擎将来多一个数据文件，这里自动带上。
 * - **这里**只负责容器：把一个 Uri 变成 zip、把一个 zip 变回一堆文件。
 *
 * 为什么容器不放进 Rust：core 的依赖政策是「只有 `tyme4rs`」，zip 要么引库要么
 * 手写几百行，而这活 `java.util.zip` 两行就干完了。
 *
 * ## 导入是「恢复」，不是「合并」
 *
 * 先清掉所有数据文件，再写包里的那些。理由：合并需要「新盖旧还是旧盖新」「数组
 * 里同名词算谁的」这类规则，每一条都能吵；而用户说「导入备份」时想要的一定是
 * **回到那个时间点**。代价是包不完整就等于删数据，所以解包**全部在内存里完成**
 * 之后才动手写盘（见 [import]）。
 */
object Backup {

    private const val TAG = "FeiheIme"

    /**
     * 单个文件的上限。
     *
     * 正常最大的个人词库是几 MB。设上限是为了挡「解压炸弹」：一个 1 KB 的包能解出
     * 几 GB，把手机存储写满，而用户只会看到「导入中…」然后磁盘满了。
     */
    private const val MAX_ENTRY_BYTES = 32L * 1024 * 1024

    /** 建议文件名：`feihe_bak_20261105.zip`。按日期取，同一天导两次由系统自动加 (1)。 */
    fun suggestedName(): String = "feihe_bak_" + stamp() + ".zip"

    /** 日期戳 `20261105`。词库导出也用同一份，省得两处各写一遍格式化。 */
    fun stamp(): String = SimpleDateFormat("yyyyMMdd", Locale.US).format(Date())

    /**
     * 导出到 `uri`，返回一句人话摘要（给 Toast 用）。
     *
     * 失败抛 [IllegalStateException]，消息是能给用户看的中文——设置页直接把它显示出来，
     * 不吞成「导出失败」四个字：写不进去的原因（空间、权限、只读位置）差别很大。
     */
    fun export(context: Context, uri: Uri): String {
        val names = NativeEngine.dataFiles()
        check(names.isNotEmpty()) { "引擎未就绪，读不到数据文件清单" }

        val dir = context.filesDir
        var count = 0
        var bytes = 0L

        openOut(context, uri).use { raw ->
            ZipOutputStream(BufferedOutputStream(raw)).use { zip ->
                // 说明文件放第一个：用解压工具打开时一眼能看到这是什么、哪个版本导的
                zip.putNextEntry(ZipEntry("MANIFEST.txt"))
                zip.write(NativeEngine.manifest().toByteArray())
                zip.closeEntry()

                for (name in names) {
                    val f = File(dir, name)
                    if (!f.isFile) continue
                    val data = f.readBytes()
                    zip.putNextEntry(ZipEntry(name))
                    zip.write(data)
                    zip.closeEntry()
                    count++
                    bytes += data.size
                }
            }
        }
        Log.i(TAG, "导出备份：$count 个文件 / $bytes 字节")
        if (count == 0) {
            return "这个设备上还没有可备份的数据（导出了一个只有说明文件的空包）"
        }
        return "已导出 $count 个文件（${sizeText(bytes)}）"
    }

    /**
     * 从 `uri` 导入，返回一句人话摘要。
     *
     * 顺序是刻意的：**先把整个包读进内存并校验完，再动手删旧的**。反过来的话，
     * 一个坏包会在「已经删掉用户数据」之后才发现读不出来——那时候连回滚的材料都没了。
     *
     * 写盘也分两步：新内容先写到 `<名字>.importing`，全部写完再删旧文件、改名。
     */
    fun import(context: Context, uri: Uri): String {
        val allowed = NativeEngine.dataFiles().toSet()
        check(allowed.isNotEmpty()) { "引擎未就绪，读不到数据文件清单" }

        // 1. 解包到内存，只认白名单里的名字
        val payload = LinkedHashMap<String, ByteArray>()
        openIn(context, uri).use { raw ->
            ZipInputStream(BufferedInputStream(raw)).use { zip ->
                while (true) {
                    val entry = zip.nextEntry ?: break
                    // 目录项直接跳过；名字必须**一字不差**地等于白名单里的一项，
                    // 于是 `../config.ini`、`sub/config.ini` 这类路径天然进不来。
                    if (!entry.isDirectory && entry.name in allowed) {
                        payload[entry.name] = readLimited(zip, entry.name)
                    }
                    zip.closeEntry()
                }
            }
        }
        check(payload.isNotEmpty()) {
            "这个压缩包里没有飞鹤的数据文件（应该包含 ${allowed.joinToString("、")}）"
        }

        // 2. 新内容先落成临时文件
        val dir = context.filesDir
        val staged = ArrayList<Pair<File, File>>()
        try {
            for ((name, data) in payload) {
                val tmp = File(dir, "$name.importing")
                tmp.writeBytes(data)
                staged.add(tmp to File(dir, name))
            }
        } catch (e: Exception) {
            staged.forEach { runCatching { it.first.delete() } }
            throw IllegalStateException("写入失败（存储空间不足？）：${e.message}", e)
        }

        // 3. 清空 → 改名。删的是「所有数据文件」，不是「包里有的那些」：
        //    导入 = 恢复到这个备份，包里没有的项就该回到默认，而不是留着旧的。
        for (name in allowed) {
            val f = File(dir, name)
            if (f.exists() && !f.delete()) {
                staged.forEach { runCatching { it.first.delete() } }
                throw IllegalStateException("删不掉 ${f.name}（可能被别的进程占用）")
            }
        }
        // user.bin 是 user_dict.txt 的编译产物，不进备份也不留旧的：留着它的话
        // 引擎会优先用它，导入的个人词库要等到下次编译才生效——表现为「导入没反应」。
        File(dir, "user.bin").delete()

        for ((tmp, dst) in staged) {
            if (!tmp.renameTo(dst)) {
                // 改名失败时内容已经在 tmp 里，直接复制过去当兜底（同一分区，很少见）
                dst.writeBytes(tmp.readBytes())
                tmp.delete()
            }
        }

        // 4. 让引擎立刻用上新数据
        NativeEngine.reloadAll()
        Log.i(TAG, "导入备份：${payload.keys}")
        return "已导入 ${payload.size} 个文件，已生效"
    }

    // ── 内部 ────────────────────────────────────────────────────────────

    private fun openOut(context: Context, uri: Uri): OutputStream =
        context.contentResolver.openOutputStream(uri, "wt")
            ?: throw IllegalStateException("无法写入所选位置（换一个目录试试）")

    private fun openIn(context: Context, uri: Uri): java.io.InputStream =
        context.contentResolver.openInputStream(uri)
            ?: throw IllegalStateException("打不开这个文件（可能已被移动或删除）")

    /** 读一个条目，超过 [MAX_ENTRY_BYTES] 就报错——不先问大小，直接边读边数。 */
    private fun readLimited(zip: ZipInputStream, name: String): ByteArray {
        val out = java.io.ByteArrayOutputStream()
        val buf = ByteArray(64 * 1024)
        var total = 0L
        while (true) {
            val n = zip.read(buf)
            if (n <= 0) break
            total += n
            if (total > MAX_ENTRY_BYTES) {
                throw IllegalStateException("备份里的 $name 太大（超过 ${sizeText(MAX_ENTRY_BYTES)}）")
            }
            out.write(buf, 0, n)
        }
        return out.toByteArray()
    }

    private fun sizeText(bytes: Long): String = when {
        bytes >= 1024 * 1024 -> String.format(Locale.US, "%.1f MB", bytes / 1048576.0)
        bytes >= 1024 -> String.format(Locale.US, "%.0f KB", bytes / 1024.0)
        else -> "$bytes 字节"
    }
}
