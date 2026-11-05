package com.feihe.ime.voice

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import android.os.Handler
import android.os.Looper
import android.util.Log
import com.k2fsa.sherpa.onnx.Vad

/**
 * 按住说话：录音 + VAD 端点检测 + 离线识别 + 上屏。
 *
 * ## 手势决定
 *
 * **按住麦克风 = 录音，松手 = 识别上屏**。不做"点一下开始、再点一下停"：
 * 那种两态按钮用户不知道现在是录还是没录，而输入法窗口没有状态栏可以放红灯。
 * 也不要自动断句上屏多段——一句话说完松手就行，多段上屏会让用户来不及看。
 *
 * VAD 只负责**裁掉首尾静音**（`front().samples` 取语音段），不决定"什么时候结束"：
 * 结束由松手决定。固定时长截断是方案 §4 明令禁止的，这里不设最大时长，
 * 只设一个宽松的 [MAX_SAMPLES] 防止忘了松手把内存吃光。
 *
 * ## 线程
 *
 * AudioRecord 的读循环在后台线程；结果通过主线程 Handler 回调 [onResult]。
 * `withEngine` 内部串行化，重复长按不会并发 decode。
 */
class VoiceRecorder(
    private val ctx: Context,
    private val onState: (State) -> Unit,
    private val onResult: (String) -> Unit,
) {
    enum class State { IDLE, LOADING, RECORDING, RECOGNIZING, NO_PERMISSION, NO_MODEL, FAILED }

    private val main = Handler(Looper.getMainLooper())
    private var thread: Thread? = null

    @Volatile
    private var recording = false

    /** 16kHz 单声道约 60 秒。只是防呆上限，不是"固定时长截断"。 */
    private val maxSamples = VoiceEngine.SAMPLE_RATE * 60

    fun hasPermission(): Boolean =
        ctx.checkSelfPermission(Manifest.permission.RECORD_AUDIO) ==
            PackageManager.PERMISSION_GRANTED

    /** 按住：开始录。电源/模型没就绪时直接给状态，不启动线程。 */
    fun start() {
        if (thread?.isAlive == true) return
        if (!hasPermission()) {
            onState(State.NO_PERMISSION)
            return
        }
        if (!VoiceEngine.modelReady(ctx)) {
            onState(State.NO_MODEL)
            return
        }
        recording = true
        thread = Thread { run() }.also {
            it.name = "feihe-voice"
            it.start()
        }
    }

    /** 松手：停止录音，剩下的交给识别。 */
    fun stop() {
        recording = false
    }

    private fun run() {
        main.post { onState(State.LOADING) }
        // 先确保模型加载完（81MB 冷加载几秒），LOADING 状态就是给这几秒看的。
        VoiceEngine.withEngine(ctx) { _, _ -> Unit } ?: run {
            main.post { onState(State.FAILED) }
            return
        }

        val bufferSize = AudioRecord.getMinBufferSize(
            VoiceEngine.SAMPLE_RATE,
            AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_16BIT,
        ).let { maxOf(it, VoiceEngine.VAD_WINDOW * 2) }

        val record = try {
            AudioRecord(
                MediaRecorder.AudioSource.MIC,
                VoiceEngine.SAMPLE_RATE,
                AudioFormat.CHANNEL_IN_MONO,
                AudioFormat.ENCODING_PCM_16BIT,
                bufferSize,
            )
        } catch (e: SecurityException) {
            main.post { onState(State.NO_PERMISSION) }
            return
        }
        if (record.state != AudioRecord.STATE_INITIALIZED) {
            record.release()
            main.post { onState(State.FAILED) }
            return
        }

        // VAD 是全局复用的，每次录音前 reset，否则上一段的端点状态会带进来。
        val speech = ArrayList<Float>()
        val pcm = ShortArray(VoiceEngine.VAD_WINDOW)
        try {
            record.startRecording()
            main.post { onState(State.RECORDING) }
            while (recording && speech.size < maxSamples) {
                val n = record.read(pcm, 0, pcm.size)
                if (n <= 0) continue
                val frame = VoiceEngine.pcm16ToFloat(pcm.copyOf(n))
                VoiceEngine.withEngine(ctx) { _, vad ->
                    vad.acceptWaveform(frame)
                    while (!vad.empty()) {
                        val seg = vad.front()
                        speech.addAll(seg.samples.asList())
                        vad.pop()
                    }
                } ?: break
            }
            record.stop()
            // 松手后把 VAD 尾部剩余的一段吐出来，否则最后一个字常被吃掉。
            VoiceEngine.withEngine(ctx) { _, vad ->
                vad.flush()
                while (!vad.empty()) {
                    speech.addAll(vad.front().samples.asList())
                    vad.pop()
                }
                vad.reset()
            }
        } catch (e: Throwable) {
            Log.e(TAG, "录音异常：${e.message}")
            main.post { onState(State.FAILED) }
            return
        } finally {
            record.release()
        }

        if (speech.isEmpty()) {
            main.post { onState(State.IDLE) }
            return
        }

        main.post { onState(State.RECOGNIZING) }
        val samples = speech.toFloatArray()
        val text = VoiceEngine.withEngine(ctx) { recognizer, _ ->
            val stream = recognizer.createStream()
            try {
                stream.acceptWaveform(samples, VoiceEngine.SAMPLE_RATE)
                recognizer.decode(stream)
                recognizer.getResult(stream).text
            } finally {
                stream.release()
            }
        }
        main.post {
            val raw = text.orEmpty()
            if (raw.isBlank()) {
                onState(State.FAILED)
            } else {
                // paraformer 不出标点，句末补一个（规则保守，见 VoicePunctuator）。
                onResult(VoicePunctuator.punctuate(raw.trim()))
                onState(State.IDLE)
            }
        }
    }

    private companion object {
        const val TAG = "FeiheVoice"
    }
}
