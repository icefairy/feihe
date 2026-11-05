package com.feihe.ime.voice

import android.content.Context
import android.content.res.AssetManager
import android.util.Log
import com.k2fsa.sherpa.onnx.OfflineModelConfig
import com.k2fsa.sherpa.onnx.OfflineParaformerModelConfig
import com.k2fsa.sherpa.onnx.OfflineRecognizer
import com.k2fsa.sherpa.onnx.OfflineRecognizerConfig
import com.k2fsa.sherpa.onnx.SileroVadModelConfig
import com.k2fsa.sherpa.onnx.Vad
import com.k2fsa.sherpa.onnx.VadModelConfig

/**
 * 离线语音识别：sherpa-onnx + paraformer-zh-small + silero VAD。
 *
 * ## 为什么用 AssetManager 而不是把模型拷到 filesDir
 *
 * 模型 81 MB，拷一份就是双倍占用，而 `OfflineRecognizer(assetManager, config)` 本来就
 * 支持从 assets 直接读（`newFromAsset`）。`build.gradle.kts` 的 `noCompress` 已经把
 * `onnx` / `bin` / `fst` 设为 STORED，AAssetManager 能拿到连续 fd，原生层可以直接 mmap。
 * 这也是 xingyu 那份验证过的样板用的路子。
 *
 * ## 为什么懒加载 + 缓存
 *
 * 81 MB 模型在骁龙 665 上冷加载约 2.7s（实测，见 `docs/Android-方案.md`）。放在按键
 * 回调里同步初始化 = 按住说话两秒没反应，用户会以为坏了。所以第一次用时在后台线程建
 * 一次，之后复用；**识别器不是线程安全的**，所有调用都走 [withEngine] 串行化。
 *
 * ## 声纹 / 热词 / 标点模型一律不要
 *
 * 项目方案 §4 明确砍掉声纹。paraformer 不出标点，句末那一个字符由 [VoicePunctuator]
 * 补（13 项 JVM 单测），不引 280 MB 的 CT-Transformer。
 */
object VoiceEngine {

    private const val TAG = "FeiheVoice"

    /** assets 里的路径。和 `tools/fetch_asr_model.py` 的落点一致。 */
    private const val ASR_MODEL = "asr/model.int8.onnx"
    private const val ASR_TOKENS = "asr/tokens.txt"
    private const val VAD_MODEL = "silero_vad.onnx"

    /** VAD 窗口。和 `SileroVadModelConfig.windowSize` 必须一致，否则端点判定会偏。 */
    const val VAD_WINDOW = 512
    const val SAMPLE_RATE = 16_000

    @Volatile
    private var recognizer: OfflineRecognizer? = null

    @Volatile
    private var vad: Vad? = null

    @Volatile
    private var loadError: String? = null

    /** 模型文件齐不齐。设置页/状态条据此显示"模型未就绪"而不是点了没反应。 */
    fun modelReady(ctx: Context): Boolean = try {
        val present = ctx.assets.list("asr")?.toSet() ?: emptySet()
        "model.int8.onnx" in present && "tokens.txt" in present &&
            VAD_MODEL in (ctx.assets.list("")?.toSet() ?: emptySet())
    } catch (e: Exception) {
        false
    }

    /** 上一次加载失败的原因，`null` = 没试过或成功。只给诊断用。 */
    fun lastError(): String? = loadError

    /**
     * 同步初始化（**必须在后台线程调**）。幂等；失败后记下原因，下次还会重试。
     */
    @Synchronized
    private fun ensure(ctx: Context) {
        if (recognizer != null && vad != null) return
        val assets: AssetManager = ctx.applicationContext.assets
        try {
            if (vad == null) {
                vad = Vad(
                    assetManager = assets,
                    config = VadModelConfig(
                        sileroVadModelConfig = SileroVadModelConfig(
                            model = VAD_MODEL,
                            threshold = 0.5f,
                            minSilenceDuration = 0.25f,
                            minSpeechDuration = 0.25f,
                            windowSize = VAD_WINDOW,
                        ),
                        sampleRate = SAMPLE_RATE,
                        numThreads = 1,
                        provider = "cpu",
                        debug = false,
                    ),
                )
            }
            if (recognizer == null) {
                val model = OfflineModelConfig(
                    paraformer = OfflineParaformerModelConfig(model = ASR_MODEL),
                    tokens = ASR_TOKENS,
                    modelType = "paraformer",
                    numThreads = 2,
                    debug = false,
                    provider = "cpu",
                )
                val config = OfflineRecognizerConfig(
                    modelConfig = model,
                    decodingMethod = "greedy_search",
                )
                recognizer = OfflineRecognizer(assetManager = assets, config = config)
            }
            loadError = null
            Log.i(TAG, "模型加载完成")
        } catch (e: Throwable) {
            // 原生层加载失败可能是 UnsatisfiedLinkError / RuntimeException，
            // 不能让它冒到按键回调里把输入法带崩。
            loadError = e.message ?: e.javaClass.simpleName
            Log.e(TAG, "模型加载失败：${loadError}")
        }
    }

    /**
     * 在后台线程跑一段用到识别器的逻辑。
     *
     * 串行化是必须的：`OfflineRecognizer` / `Vad` 内部持有原生指针，两个按键同时长按
     * 会并发 decode，ptr 会被另一个线程 release 掉。
     */
    @Synchronized
    fun <T> withEngine(ctx: Context, block: (OfflineRecognizer, Vad) -> T): T? {
        ensure(ctx)
        val r = recognizer ?: return null
        val v = vad ?: return null
        return try {
            block(r, v)
        } catch (e: Throwable) {
            loadError = e.message ?: e.javaClass.simpleName
            Log.e(TAG, "识别失败：${loadError}")
            null
        }
    }

    /**
     * 把一段 PCM16 转成 [-1,1] 的浮点样本。
     *
     * 纯函数，单独出来是为了能跑 JVM 单测——`AudioRecord` 给的是 Short，
     * 除错一个数（32768 vs 32767、忘了归一化）在真机上的表现是"识别全是噪声"，
     * 而这类错误只靠听根本定位不到。
     */
    fun pcm16ToFloat(pcm: ShortArray): FloatArray =
        FloatArray(pcm.size) { i -> pcm[i] / 32768.0f }
}
