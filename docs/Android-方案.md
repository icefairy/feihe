# 飞鹤输入法 · Android 平台方案

> 规划日期：2026-11-05
> 参考：`d:\codes\qingjian` 的 `apps/android`（Rust core + JNI + Kotlin 壳的成熟跨平台样例）、
> `d:\codes\xingyu` 的 `m1`（sherpa-onnx 离线 ASR 的验证样板）。
> 目标：把小鹤双拼 / 音形 / 联想做成 Android 输入法（IME），并带上离线语音输入。
>
> **总目标：替换搜狗输入法。** 只有覆盖手机、且日常能力对齐，才谈得上"替掉"。
> 所以 Android 不是 Windows 端的附属实验，而是同一份 `feihe-core` 的一等公民平台壳；
> 搜狗有而飞鹤还缺的（云同步、词库规模、皮肤商店、手写）见 README 开头的差距表。
>
> 进度（2026-11-05 夜）：M1–M3 完成，M4 语音代码完成待真机验收；
> 逐段选择 / 上档锁定 / 数字选词 / 联想打磨 / 候选个数设置均已补齐。
> 逐条状态见 [`后续待办.md`](后续待办.md)，验收序列见 [`验证清单.md`](验证清单.md) 的 Android 分组。

---

## 0. 结论先说

**飞鹤 Android 不是「把 TSF 输入法移植过去」，而是「给现有的纯逻辑引擎换一个平台壳」。**

具体分工：

| 层 | 来源 | 说明 |
|---|---|---|
| 引擎（双拼 / 音形 / 词库 / 联想 / 短语 / 模糊音 / 纠错 / 简繁 / 标点） | **飞鹤自己的 Rust 代码**，抽成 `feihe-core` | 算法一行不用重写，但要做一次 crate 拆分 |
| 平台壳骨架（IME 生命周期 / 键盘 / 候选条 / 设置 / 权限 / 剪贴板） | **抄青简 android 的结构** | 它踩过的 Android 坑（键盘重建、无痕输入、输入框类型）都能省掉 |
| 离线语音（ASR） | **星语 m1 的 sherpa-onnx 组合** | **内置** paraformer-zh-small（79 MB，int8）+ silero VAD；无标点，句末补一个（SenseVoice 228 MB 作为将来可选下载） |

一句话：**引擎是飞鹤的，壳是青简验证过的，耳朵是星语的。**

风险不在技术路线，在**工作量**——尤其是第一步的 crate 拆分，它同时要保证 Windows 端行为一个字都不变。

---

## 1. 三个项目的可复用资产

| | 飞鹤 feihe | 青简 qingjian | 星语 xingyu |
|---|---|---|---|
| 现状 | Windows TSF 输入法，Rust，单平台 | Rust core + 桌面前端 + **Android 壳**（237 个文件的成熟实现） | 儿童语音陪伴 App 的 M1 验证 |
| 引擎 | 双拼 / 音形 / 联想 / 词库 / 短语，全在 `src/*.rs` | `qingjian-core`，双拼 / 音形 / 五笔 / 云联想 | 无引擎 |
| 语音 | 无 | 有（火山云 + 本地离线预留接口 `Transcriber`） | **sherpa-onnx：SenseVoice ASR + CAM++ 声纹 + VAD，有可跑的 Android demo** |
| 跨平台 | 无抽象（`cdylib` 直接绑 TSF） | **core 已抽象，Android/桌面同一套** | 单平台 |
| 许可 | MIT | — | — |

### 能从青简直接借鉴的（按价值排序）

1. **`InputMethodService` 的完整生命周期处理**：`onStartInputView` 每次重量键盘高度（三键导航栏会盖住最下一排）、`onStartInput` 按 `EditorInfo` 换键盘类型、`onFinishInput` 清会话。
2. **JNI 边界设计**：`Native.kt` 一个 handle（`jlong`）指向 Rust 侧会话，所有操作 `handle + 参数 → 帧（Frame）`。飞鹤照这个形状做，接口数量能控制在 20 个以内。
3. **候选条渲染**：青简用 Rust 渲染器出位图、Kotlin 贴图；飞鹤可以直接用 Compose 画（候选 <= 9 个，不需要跨平台渲染器）。
4. **隐私开关**：`IME_FLAG_NO_PERSONALIZED_LEARNING` → 会话置私密，不学习不联想；密码框按 `TYPE_TEXT_VARIATION_PASSWORD` 判定——飞鹤 Windows 端已有 `secure.rs` 的等价逻辑，规则可以平移。
5. **模型按需下载**：青简的万象模型就是"默认不下载、用的时候才下、校验 SHA256"。语音模型体积大，必须走这条路。

### 能从星语直接借鉴的

1. **模型选型已经验证过**：SenseVoiceSmall 的 ONNX 版，int8 实测 **228 MB**，非流式、自带标点与数字规整。体积决定了它**不能随包**，走 app 内下载（源只用 ModelScope，实测 4.9 MB/s）。
2. **VAD 必须单独存在**：`sherpa-onnx` 的 silero VAD（约 1 MB）决定"说完了没有"，不能靠固定时长截断。
3. **集成样板**：`m1/android-app` 就是 sherpa-onnx 官方 Android demo，`OfflineRecognizer.kt` / `OfflineStream.kt` 是现成的 Kotlin 调用层，`jniLibs` 的 so 组织方式照抄即可。
4. **不需要声纹**：星语的 CAM++ 是为了"只认孩子的声音"，输入法不需要。这一块直接砍掉，省 7 MB 和一条权限链路。

### 不能借鉴的

- **星语的"打电话模式"**：那是常驻监听 + 声纹门禁 + TTS + 打断，是陪伴产品的形态。输入法的语音是**按需触发、说完即走**，两套交互完全不同。
- **青简的翻译 / 学习语言 / 剪贴板历史**：那是另一个产品的功能面。飞鹤不做。

---

## 2. 目标架构

```
┌──────────────────────────────────────────────────────────┐
│ Kotlin 壳（app/）                                         │
│                                                          │
│  FeiheInputMethodService : InputMethodService            │
│   ├─ KeyboardView（26 键 / 双拼键帽 / 长按弹符号）        │
│   ├─ CandidateBar（候选条，Compose 画）                   │
│   ├─ VoicePanel（麦克风面板：按下说话 / 波形 / 结果）      │
│   └─ SettingsActivity（设置页，写 SharedPreferences）      │
│                                                          │
│  Voice：AudioRecord → sherpa-onnx VAD → ASR → 文本        │
└───────────────────────┬──────────────────────────────────┘
                        │ JNI（handle + 调用 → Frame）
┌───────────────────────▼──────────────────────────────────┐
│ Rust：feihe-core（新增 crate，Android 与 Windows 共用）    │
│                                                          │
│  Engine                                                   │
│   ├─ session：编码 / 分词 / 候选 / 上屏                    │
│   ├─ dict：词库（base.bin + 用户词库）                     │
│   ├─ shuangpin / assist / fuzzy / correct                 │
│   ├─ freq / phrase / pin / demote / custom / next         │
│   ├─ zhconv（简繁，需替换 LCMapStringEx）                  │
│   ├─ number / calc / quick                                │
│   └─ commit_voice(text)：语音结果上屏（不记词频）          │
└──────────────────────────────────────────────────────────┘
```

### 关键取舍

**语音结果不走引擎的候选流程。** 识别出「今天天气怎么样」，直接 `commitText` 上屏，不经过拼音组句、不进候选、不学词频。理由：

- 语音是**另一条输入通道**，不是"用嘴打拼音"。让它进组句只会制造"语音结果被候选覆盖"的怪事。
- 青简的 `Engine::commit_voice` 就是这么划的（识别结果不记词频、不算选词、只记输入统计）。
- 好处是 ASR 可以完全留在 Kotlin 层，Rust 只暴露一个 `commitVoice(text)`——**语音和引擎彻底解耦**，模型换、识别方换，引擎一行不用动。

代价：语音结果不能和拼音混输（说一半再打一半）。第一版不需要。

---

## 3. 引擎搬迁：哪些能搬、哪些要重写

飞鹤现在是 49 个 `src/*.rs`。按"能不能进 core"分三类。

### 3.1 直接搬（纯逻辑，只改 `use`）

| 文件 | 内容 | 备注 |
|---|---|---|
| `shuangpin.rs` | 双拼编码 / 音节切分 | 纯函数 |
| `assist.rs` + `assist.txt` | 小鹤音形 / 自然码辅助码 | 纯查表 |
| `fuzzy.rs` | 模糊音 9 条 | 纯函数 |
| `correct.rs` | 拼写纠错变体 | 纯函数 |
| `dict.rs` | 词库查询 / 排序 / 模糊 / 纠错候选 | 依赖 `dictbin`，见下 |
| `dictbin.rs` / `dictenc.rs` | 词库二进制格式与预编码 | 格式本身可搬 |
| `brief.rs` / `brief2.rs` | 一简 / 二简 | 纯查表 |
| `freq.rs` | 动态词频 | 依赖 `config::write_atomic`，改路径注入 |
| `phrase.rs` | 自造词 | 同上 |
| `pin.rs` / `demote.rs` / `custom.rs` | 置顶 / 降权 / 自定义短语 | 同上 |
| `next.rs` | 联想（含内置表） | 完全纯逻辑 |
| `number.rs` / `calc.rs` | 中文数字 / 算式 | 纯函数 |
| `quick.rs` | 快捷输入 | **已搬**（2026-02）：`core/src/quick.rs`，`GetLocalTime` 换 `core::clock`，日历/农历/干支/节气走 `tyme4rs`——唯一第三方依赖 |
| `hint.rs` | 易错词读音 | 纯查表 |
| `sogou.rs` | 搜狗词库导入 | 纯解析（文件选择在壳里） |

### 3.2 要改写（依赖 Windows API）

| 文件 | 现状 | Android 方案 |
|---|---|---|
| `textservice.rs` | TSF 的全部实现（3000+ 行） | **不搬**。它做的事（组句状态机、按键路由、上屏）要拆成 `session.rs`（纯状态机，可搬）+ 壳里的事件转发。**这是最大的一块工作量** |
| `candidate.rs` | 自绘候选窗（Win32 窗口） | Kotlin 画 |
| `statusbar.rs` | 状态浮窗（Win32） | 不需要（Android 输入法自带切换） |
| `settings.rs` | Win32 原生控件设置窗 | Kotlin 设置页，读写同一份配置语义 |
| `softkbd.rs` | GDI 自绘软键盘 | 用系统键盘，不需要 |
| `theme.rs` / `font.rs` | GDI 调色与字体 | Kotlin 主题 |
| `insert.rs` | 往别的窗口写字 | `InputConnection.commitText` |
| `secure.rs` | 密码框判定 | `EditorInfo.inputType` |
| `fullscreen.rs` | 全屏检测 | 不需要 |
| `clipboard.rs` | 剪贴板反查 | `ClipboardManager` |
| `uielement.rs` | TSF UILess | 不需要 |
| `zhconv.rs` | **`LCMapStringEx`** | **必须换**：Android 没有这个 API。用 OpenCC 的 `t2s` 表（青简的 `opencc` 就是这么做的），或只支持"简→繁常用字"的轻表 |
| `config.rs` | `%APPDATA%` 路径 + 原子写 | 路径由壳注入（`filesDir`），原子写保留 |
| `diag.rs` | 日志到安装目录 | 换 `logcat` |
| `classfactory.rs` / `clsid.rs` / `dllcount.rs` / `lib.rs` | DLL 导出与 COM | 换 `jni` 导出 |

### 3.3 crate 拆分（第一步，也是风险最大的一步）

```
feihe/                       现有仓库
├── core/                    ← 新增 crate：feihe-core
│   ├── src/（3.1 那些文件 + session.rs）
│   └── Cargo.toml           no_std 不必要，但**不允许依赖 windows**
├── windows/                 ← 新增 crate：feihe-win（现在的 src/ 剩下部分）
├── android/                 ← 新增：apps/android（Kotlin + jni 胶水）
└── src/                     ← 暂时保留，逐步搬空
```

**硬约束**：`feihe-core` 里出现任何 `windows::` 都算编译失败。用 `cargo denmy`（或 `grep`）在 CI 里卡住。

**为什么这步最难**：`textservice.rs` 里"组句状态机"和"TSF 调用"是混在一起的（1800 行的 `OnKeyDown`）。拆分时最容易出的错是**顺手改了逻辑**——Windows 端有 313 项单测和一份验证清单，所有行为必须逐条对得上。策略：

1. 先做**纯剪切**：`Session` 只搬状态和决策（`fn key(&mut self, k: Key) -> Action`），不搬任何 `ITf*`。
2. 每个搬出来的函数保留原注释和单测，测试跟着搬。
3. Windows 壳改成"把键事件喂给 Session，执行返回的 Action"。
4. **判定标准**：313 项单测全绿 + 验证清单里 M3.x 全过 ≠ 完成；还要真机打字对比一遍（用同一个词库、同一串输入，输出必须逐字相同）。

工作量粗估：core 拆分 + Windows 回归约 **2–3 周**（`textservice.rs` 占一半）。

---

## 4. 离线语音（ASR）方案

### 4.1 模型选型：实测体积，不是估的

2026-11-05 实测（不是文档里传抄的数字）：

| 模型 | 形态 | int8 实际体积 | 标点 | 中英混读 | 冷加载（骁龙 665） |
|---|---|---|---|---|---|
| **SenseVoiceSmall** | 非流式 | **228.2 MB**（239,233,841 B） | **有**（需 `use_itn=1`） | 好 | 未测（非流式架构，预计 3–10 s） |
| paraformer-zh-small | 非流式 | **79 MB** | **无** | 中英双语可以 | RTF 0.076（很快） |
| Zipformer Multi-zh | 流式 | 72.8 MB | 无 | 大写字母可以，整词不行 | 10.1 s |
| Zipformer 14M | 流式 | 24.2 MB | 无 | 差（字母念成汉字） | 2.7 s |
| X-ASR 双语标点 | 流式 | 161.4 MB | **自带** | 好 | 13.4 s |
| silero VAD | — | 0.2 MB | — | — | — |

（Zipformer / X-ASR 几行来自一篇 Redmi Note 8 / 骁龙 665 的真机横评；SenseVoice 的大小
是我直接从 ModelScope 拉 `Content-Range` 读出来的，`tokens.txt` 另计 308 KB；
paraformer-zh-small 的数字来自 sherpa-onnx 官方模型页。）

#### 「星语里那个 80 多 MB」= paraformer-zh-small，不是 SenseVoice

`csukuangfj/sherpa-onnx-paraformer-zh-small-2024-03-09` 的 `model.int8.onnx` 正好 **79 MB**，
而且星语那份 sherpa-onnx 官方 demo 的 `OfflineRecognizer.kt:450` 里就有这个预设名。
所以「80 多 MB」这个印象是真的，但它对应的是**另一个模型**。

另外要说清楚：**星语的 app 里其实从来没有 ASR 模型**。`m1/android-app/app/src/main/assets/`
只有 `3dspeaker campplus`（27 MB，声纹）和 `silero_vad.onnx`（629 KB）；ASR 是
`.gitignore` 掉的 `m1/models/`，脚本里引的是 WSL 里的
`sherpa-onnx-sense-voice-…-2024-07-17`。调研文档里那句「234M int8 量化（~60-120MB）」
是**估算**，实测 228 MB——我一开始也照抄了这个估算，后来才发现。

#### 为什么 79 MB 那个不能用：它不出标点

官方模型页给的全部示例输出都是连续字符串，一个标点都没有：

```
对我做了介绍啊那么我想说的是呢大家如果对我的研究感兴趣呢
重点呢想谈三个问题首先呢就是这一轮全球金融动荡的表现
```

paraformer 系全都这样（标点要靠外挂 CT-Transformer，约 62 MB，79 + 62 = 141 MB，
比 SenseVoice 还差）。输入法的语音是往输入框塞**一段能直接发的文本**，不是转写稿；
没有标点就得用户自己回去补，那还不如不做语音。

SenseVoice 的标点来自 `use_itn=1`（官方示例，关 ITN 是 `开饭时间早上九点至下午五点`，
开 ITN 是 `开放时间早上9点至下午5点。`——顺带把数字也规整了）。**这个开关必须开，
默认不开。**

流式在这里反而**不是**优点：输入法的语音是按住说话、松手出字，一句话几秒钟，
中间结果闪来闪去只会干扰。非流式天生匹配这个交互。

许可：FunASR 团队在 `QwenAudio/SenseVoice` issue #286 里明确答复——FunASR 自家模型
（含 SenseVoice）**允许商用，条件是署名 + 保留模型名**。可以随包、可以分发。

### 4.2 决策：内置 79 MB 的 paraformer-zh-small，接受无标点（B 方案）

**拍板结果：模型随包，用一个真正能离线工作的输入法换掉「体积漂亮」。**

装 SenseVoice 是 ~245 MB 的输入法，用户看到安装包大小就划走了；而 79 MB 的
paraformer-zh-small 让 APK 落在 **80.0 MiB**（已实测，debug 包 83,936,733 字节；
release 去掉符号和压缩后还会更小）——比搜狗（装机后 100 MB 上下）小，在「输入法」
这个品类里不算离谱，而且是**装完就能说话、一次网络都不用**。

代价是明确的：**它不出标点**。官方示例输出是

```
对我做了介绍啊那么我想说的是呢大家如果对我的研究感兴趣呢
```

这不是第一版能绕过去的（paraformer 系全这样，外挂 CT-Transformer 要 280.7 MB，
见 §4.6），所以第一版**只做一件事补偿：句末补一个标点**。中间的逗号不猜——
猜逗号要理解语义，猜错比不加更让人烦躁。

#### 4.2.1 句末补标点（已实现，13 项单测）

代码在 `android/app/src/main/java/com/feihe/ime/voice/VoicePunctuator.kt`，
测试在 `app/src/test/.../VoicePunctuatorTest.kt`，`./gradlew testDebugUnitTest` 可跑
（纯 JVM，不需要真机）。

规则**故意保守**：

| 输入 | 输出 | 说明 |
|---|---|---|
| `今天天气不错` | `今天天气不错。` | 正常补句号 |
| `你去吗` | `你去吗？` | **只有「吗」判疑问句** |
| `我在家呢` | `我在家呢。` | 「呢」不判问号——陈述句里太常见，判错更糟 |
| `你好。` / `走不走？` / `他说：“你好”` | 原样 | 已有标点一个字不动，**幂等** |
| `好` | `好` | 单字不加，聊天里很怪 |
| `hello world` | `hello world.` | 纯 ASCII 用英文句号 |
| `我今天用 main 分支` | `我今天用 main 分支。` | 含汉字就按中文标点 |

三条边界值得记住：**幂等**（重复调用不会变「。。」）、**收尾引号不补**
（`他说：“你好”` 后面再加句号是错的）、**单字不补**。

#### 4.2.2 模型从哪来（B 方案唯一的工作源）

79 MB 的 onnx 不能进 git（每次 clone 都背一遍，历史里还删不掉），也不该让用户去下。
所以它是**构建输入**：`tools/fetch_asr_model.py` 拉下来放进
`android/app/src/main/assets/asr/`，该目录已 gitignore。

实测的三条路：

| 源 | 结果 |
|---|---|
| **hf-mirror.com** | **663 KB/s，HTTP 206（支持 Range）**，79 MB 约 2 分钟 ← **用这个** |
| ModelScope | **没有** `csukuangfj/sherpa-onnx-paraformer-zh-small-2024-03-09` 这个仓库（`/repo/files` 返回 record not found） |
| GitHub Releases | `http=000`，完全不通 |

脚本先把内容写成 `.part` 再改名——中断不会留下一个"看起来完整"的半截 onnx。
拉完把打印出来的 SHA256 填进脚本里的 `EXPECTED`，以后 `--verify` 能挡住上游换文件。

#### 4.2.3 可选升级：换成 SenseVoice（本期不做，但数据先留着）

如果将来觉得"没标点"不能忍，升级路径是**下载** 228 MB 的 SenseVoice 而不是换包，
因为 228 MB 不该进安装包。这一段的实测数据和流程设计已经验证过，等要用时直接照做：

- **下载源只有 ModelScope**：2026-11-05 本机实测 **4.89 MB/s，HTTP 206（支持 Range
  → 断点续传）**，228 MB 约 47 秒；**GitHub Releases `http=000` 完全不通**；
  Gitee 单文件放不下 228 MB。
- ModelScope 响应头 `X-Linked-ETag` 就是 SHA256，可直接当校验值。
- **别长期依赖社区转存**（`poloniumrock/SenseVoiceSmallOnnx` 随时可能被删）：
  真要做就把 onnx 传进自己的 ModelScope 仓库，钉死大小 + SHA256，加载前校验。
- 流程按「会失败」设计：先问再下（移动数据二次确认）→ WorkManager 后台下载（进程被杀
  能续）→ Range 续传 → 重试 3 次 / 换备用源 / 提示手动选文件 → 空间预检 →
  下完预热 → **可删除**（228 MB 删不掉的 App 会被卸载）。
- 落地在 `filesDir/asr/` 而不是 assets：放 assets 才需要 `noCompress` 那套零拷贝，
  走文件系统没这个坑。


#### 4.2.4 「我有模型文件」这条路也留着

不是给普通用户，是给两种人：完全不想联网的（这也符合飞鹤本身的产品定位），
以及企业内网批量部署的。入口在设置页「语音输入 → 使用本地模型」，选一个目录，
校验通过就用。这条路**不联网、不校验来源**，只校验文件完整性——用户自己的文件，
他自己负责。

> 待实测：SenseVoice 在真机上的冷加载时间与 RTF。228 MB 模型 + 非流式，中低端
> 机上冷加载可能到 5–10 秒，这是「下完预热」那条存在的理由。如果实测超过 10 秒，
> 就要考虑常驻一个轻量初始化，或者第一次唤醒时给一句「正在准备语音，只需一次」。


### 4.3 集成位置：全部在 Kotlin 壳

sherpa-onnx 官方提供 Android 的 Kotlin API（星语的 `m1/android-app` 就是官方 demo 的翻版，`OfflineRecognizer.kt` / `OfflineStream.kt` 可直接参考）。所以：

```
AudioRecord(16 kHz, 单声道, PCM16)
   → VAD 切片
   → OfflineRecognizer.decode(samples)
   → 文本
   → InputConnection.commitText(text)
```

**Rust 侧只加一个 `commitVoice(text)`**，用于记录统计（今天打了多少字）。识别本身不经过 core——理由见第 2 节：语音是独立通道，不需要和拼音组句融合。

好处是这块可以**完全独立开发**：它不碰引擎、不碰键盘，做好了就能用。

### 4.4 交互

沿用青简验证过的形态（`docs/design/voice-and-swipe.md`）：

- **入口**：键盘右上角一个话筒键（不占长按空格——小鹤用户的长按空格习惯留给别的）。点了弹出语音面板。
- **面板**：盖住候选条与键盘，中间一个大的「按住说话」区域 + 实时波形；说完（VAD 判定静音 800 ms 或松手）自动识别、上屏、收起。
- **中间结果**：第一版的 paraformer-zh-small 是整段识别（非流式），所以没有"边说边出
  字"。真需要流式得换 zipformer 系（24.2–161.4 MB），但它们同样不带标点。输入法的
  语音是一句话几秒钟，等说完再出整句反而更稳，不值得为"边说边出"换架构。
- **取消**：面板上「取消」按钮，或上滑手势。

### 4.5 权限与隐私

- `RECORD_AUDIO`。输入法**不能弹系统权限对话框**（没有 Activity 上下文），必须开一个透明的 Activity 来申请——这是青简踩过的坑（`docs/notes/android.md`「要录音的识别方式先查麦克风权限，没有就开一个透明窗口弹系统权限框」）。
- 语音数据只在内存里，识别完即丢，**不落盘、不上传**。这一点要在设置页写清楚（飞鹤的产品定位就是"不联网"，语音是唯一可能让人怀疑的地方）。
- 密码框 / 无痕模式下**禁用语音入口**。

---

### 4.6 以后：能不能用很小的模型来自动加标点 / 纠错

这是明确记下来的后续研究方向，不是本期的活。先摆事实，免得重复研究。

### 4.6.1 现成的中文标点模型，一个都不小（2026-11-05 逐个查过）

| 模型 | 架构 | 实际体积 | 结论 |
|---|---|---|---|
| `sherpa-onnx-punct-ct-transformer-zh-en-vocab272727` | CT-Transformer | **280.7 MiB**（fp32） | 比 ASR 模型大 3.5 倍 |
| 同名 `-int8` | 同上 | 仓库在 HF / ModelScope **都不存在** | 只在 GitHub release，本机不通 |
| `jiangzhuo9357/fireredpunc-onnx`（`punc.q8w.onnx`） | chinese-lert-base（BERT-base 12 层） | **155.2 MiB** | Apache-2.0，但只有 5 类标点；作者说官方 int8 版会丢掉一半句末标点 |
| `42ailab/FireRedPunc-ONNX` | 同上 int8 | 仓库**不存在** | — |
| `p208p2002/zh-wiki-punctuation-restore` | BERT | 388 MiB（PyTorch，非 ONNX） | 还要自己导出 |
| **`frankyoujian/Edge-Punct-Casing`** | **CNN + BiLSTM + unigram BPE** | **28.2 MiB**（7z，**英文**） | 唯一"小架构"，但它只做英文 |
| `shibing624/macbert4csc-base-chinese`（纠错） | MacBERT-base | 390 MiB | 纠错方向同样没有小件 |

两条硬结论：

1. **现成的中文标点 ONNX，最小的是 155 MiB**（BERT-base 系）。79 MB ASR + 155 MB 标点
   = 234 MB，和直接用 SenseVoice 一个量级，那还不如直接用 SenseVoice。
2. **唯一证明"能做小"的是 CNN-BiLSTM**（Edge-Punct-Casing 英文版 28.2 MiB）。它说明
   30 MB 以内是可能的——但**中文版没有任何人发布可用的 ONNX**（那个中文派生项目在
   GitHub 上，本机访问不通，也没有体积数据）。

顺带确认：`sherpa-onnx-online-punct-en` 只做英文；`modelers.cn` 上 AXERA 那个
`punc-ct-transformer` 页面是 JS 渲染 + 要登录，拿不到文件清单，无法核实体积。

### 4.6.2 自己训一个字符级小模型，是唯一现实的路

好消息是**标点任务的语料获取成本极低**：任何一段正常的中文文本（维基、新闻、
小说、公众号）都是天然标注好的标点语料——把标点去掉，让模型学「该在哪儿加回来」。
这和语音识别完全不同（那个要有音频+转写对齐）。

架构照 Edge-Punct-Casing 那条已被验证的路走（**CNN + BiLSTM**，不要 BERT）：
字符嵌入（常用字 ~7000 × 64 维）+ 卷积 + 2 层 BiLSTM + 每位置多分类头
（无 / ，/ 。/ ？/ ！）。参数量约 50–80 万，fp32 **2–3 MB**，int8 后 **不到 1 MB**。
比英文那版 28 MB 还小，因为中文用字符表（7k）而不是 8k+ 的 BPE 词表。
CPU 上对一句 30 字做一次前向是毫秒级。

风险不在模型，在**训练数据和工程**：需要 PyTorch 环境、需要把模型导成 ONNX、
需要处理「模型加了逗号但位置不对」比不加更糟的情况（所以还得有个置信度阈值）。

### 4.6.3 纠错方向：paraformer 用不上热词偏置

按 sherpa-onnx 的实现，热词 / 上下文偏置走的是 `modified_beam_search`，那是
transducer 的解码路径；paraformer 是 CIF + 注意力解码，没有这个入口。所以
**纠错只能做后处理**，两条路：

1. **用飞鹤自己的词库做同音/近音纠偏**：98k 词条 + 词频，对识别结果做「低频词 →
   同音高频词」的替换。零体积、零训练，但容易误伤专有名词。
2. **小重打分模型**（4 MB 量级的字符级 reranker）：能给候选打分，但**必须有评测集**。

### 4.6.4 顺序建议：先攒错误样本，再谈模型

这一条比上面三条都重要。语音纠错最怕「凭感觉调」，因为你永远不知道改好还是改坏了。
正确的顺序是：

1. 先上线（79 MB + 句末标点），能用就行；
2. 在**用户明确同意**的前提下，记一份「说了什么 / 识别成什么」的本地样本（不进网络）；
3. 攒到几百条之后，人工标一遍对错，得到一个能跑的评测集；
4. 有了评测集，上面任何一个方案都能验证——没有它，做了也不知道是不是变好。


---

## 5. Android 输入法特有的坑（青简已经踩过的）

这些是 TSF 那边完全没有对应物、第一次做 Android IME 一定会撞上的：

1. **`onCreateInputView` 只调一次**。键盘高度由导航栏决定，而用户可能中途切换三键/手势导航——要在 `onStartInputView` 里重新量高度，变了就重建视图（青简原话：「三键的导航栏会盖住最下一排」）。
2. **候选条不能是 Dialog**。输入法窗口层级特殊，Dialog 会被盖住。候选条必须是 InputView 的一部分。
3. **`onStartInput` 要按 `EditorInfo.inputType` 换键盘**：电话/数字/日期类输入框直接进数字键盘；密码框不过引擎（不学习、不联想、不记忆）。
4. **`IME_FLAG_NO_PERSONALIZED_LEARNING`**：浏览器无痕标签页会带这个 flag，此时必须关掉学习（等价于飞鹤 Windows 端的"密码框不接管"）。
5. **进程会被杀**。输入法进程内存不能当存储：配置、词频、联想表都必须落盘，`onFinishInput` 时要 flush。
6. **硬件键盘**（外接蓝牙/平板磁吸键盘）走 `onKeyDown`，要单独处理——好在飞鹤的键路由逻辑本来就在 `Session` 里，复用即可。
7. **长按删除要连删**，且速度要随按住时间加速（Gboard 的手感）。
8. **输入法切换器**：Android 11+ 的输入法切换按钮有固定位置，键盘布局要给它让位。
9. **横屏全屏模式**：`onEvaluateFullscreenMode` 默认在横屏且输入框小时返回 true，此时系统会全屏接管——要关掉，否则候选条位置错乱。
10. **`commitText` 之后必须清理 composing**：Android 的 `setComposingText("")` 和 `finishComposingText` 语义不同，选错会留下下划线残影。

---

## 6. 里程碑与工作量

单人开发的粗估（不含打磨）：

| 阶段 | 内容 | 估算 | 交付判据 |
|---|---|---|---|
| **M1** | crate 拆分：`feihe-core` + `feihe-win`，`textservice.rs` 拆成 `session.rs`（纯状态机）+ 壳 | **2–3 周** | Windows 端 313 项单测全绿 + 真机逐字对比一致 |
| **M2** | Android 最小可用：IME 骨架 + 26 键键盘 + 候选条 + 双拼输入 + 上屏 + 读配置 | **3–4 周** | 能装、能打字、能选词、能标点 |
| **M3** | 功能补齐：联想 / 短语 / 置顶 / 降权 / 简繁 / 模糊音 / 纠错 / 快捷输入 | **✅ 2026-02** | 与 Windows 端功能对齐（除 Windows 专属项）。**快捷输入**：`quick.rs` 整体搬进 `feihe-core/src/quick.rs`（`GetLocalTime` → `core::clock`、`CoCreateGuid` → 时间派生模拟 v4、`fancha` → `core::fancha`），桌面壳只剩 `pub use` 门面，两端共用一份命令表；JNI 三个入口（`quickPrefixNative` / `quickCandidatesNative` / `quickCodeOkNative`）在 `android/jni/src/quick_extra.rs`，Kotlin 侧 `quickMode` 是独立状态位（不靠 `pending` 现判），快捷候选不学词不联想 |
| **M4** | 语音：sherpa-onnx 集成 + 79 MB 模型打进 assets（`noCompress`）+ 句末标点 + 语音面板 | **✅ 2026-11-05** | 代码完成：按住 🎤 / silero VAD / paraformer `newFromAsset` / 句末标点 / 设置页开关与权限。APK 已核对 5 个 `.so` + 3 个 assets 齐全、`.onnx` STORED。**交付判据仍差真机验证**（adb WiFi 掉线），见 `handoff.md` |
| **M5** | 设置页、皮肤、剪贴板、长按符号、手感打磨 | 持续 | — |

**合计约 10–14 周**（单人、全职）。其中 M1 是唯一有"改坏现有产品"风险的阶段。

如果只想先验证"飞鹤能不能上 Android"，可以砍掉 M3/M4，**M1 + M2 六周出个能打双拼的 APK**。

---

## 7. 明确不做的（范围控制）

Android 输入法的功能面很容易失控——青简的 shell 有 237 个文件，那是三年打磨的结果。飞鹤第一版必须砍：

| 不做 | 理由 |
|---|---|
| **九宫格** | 小鹤双拼是 26 键方案，九宫格要另一套 T9 模糊匹配逻辑。双拼每音节固定 2 键，26 键全键盘在手机上已经够快，做九宫格等于重新设计一套编码 |
| **滑行输入** | 青简的 `SwipeDecoder` 是给**全拼**和**英文**的（滑出一条 `zhongguo`）。双拼的键序列不是拼音串，滑行解出来对不上任何词 |
| **五笔 / 注音 / 日语** | 飞鹤的定位是小鹤双拼，不是"什么都能打" |
| **云联想 / 账号 / 翻译** | 产品定位是离线、不登录、不收集。语音模型下载是唯一的联网点，且必须用户主动点 |
| **声纹** | 那是陪孩子的产品才需要的门禁，输入法不需要 |
| **剪贴板历史** | 有用但和输入法核心无关，等 M5 再考虑 |
| **皮肤商店 / 在线词库** | 同上 |

---

## 8. 和青简 android 的关键差异

虽然抄它的架构，但有两个地方不能照搬：

### 8.1 引擎不是它的

青简的 `qingjian-core` 支持双拼、五笔、注音、日语罗马音、九宫格、云联想、翻译、学习语言——那是一个"通用输入平台"。飞鹤只有一个方案（小鹤双拼）+ 音形辅助码，**引擎更窄也更深**：

- 青简的双拼是"方案表驱动"；飞鹤的双拼有音形（形码反查、3 键直出、Tab 进辅助码）和分词（长句切段逐段选），这些是青简没有或不完全一样的。
- 所以**不能直接换用 `qingjian-core`**——那等于把飞鹤的产品差异丢掉。必须搬自己的引擎。

### 8.2 手机上双拼的真实体验问题

这是 Windows 和手机最大的差别，值得单独想清楚：

- **双拼在 26 键手机上其实很爽**：每音节 2 键，比全拼省一半击键。这是飞鹤上手机最大的卖点。
- **但候选条空间只有一行**。Windows 上飞鹤的杀手功能是"翻页摊开 5 行、一屏 45 个"，手机上放不下。手机必须靠：`1-9` 数字选词 + 左右滑候选条 + 长按展开。
- **音形辅助码在手机上价值更高**（同音字多、候选条窄），但 Tab 键在手机键盘上没有——要设计一个"辅助码键"（放在底排，或长按某个键）。
- **分词逐段选**在手机上交互较重（要左右移段），第一版可以先不做：手机上用户更倾向"一次选一个词"。

> 这几条建议在 M2 期间用真机各打 500 字再定，不要凭 Windows 的手感设计手机交互。

---

## 9. 第一步该做什么

如果现在就要动手，顺序是：

1. **先做 `feihe-core` 的拆分，但不要一次动 `textservice.rs`。**
   从最干净的模块开始搬（`shuangpin` → `fuzzy` → `correct` → `number` → `calc` → `next` → `brief`），每搬一个就让 Windows 端编译 + 跑测试。这样任何回归都能立刻定位到"刚搬的那个文件"。
2. **给 `feihe-core` 加一道依赖闸**：`Cargo.toml` 里显式声明只允许 `tyme4rs`，CI 里 `cargo tree` 出现 `windows` 就红。
3. **`config.rs` 先做路径注入**：把 `dir()` 改成 `OnceLock<PathBuf>` + `init(dir)`，Windows 侧在 `DllMain` 里初始化——这一步不做，core 里任何模块都离不开 `%APPDATA%`。
4. **`zhconv.rs` 换实现**：先做"简→繁常用字"表（约 3000 字对，几百 KB），够用且不依赖 OpenCC。真要全量再上 OpenCC 数据。
5. **Android 壳先跑通"Hello World IME"**：一个 `InputMethodService` + 一个只出"你好"的键盘，装到真机看能不能上屏。**这一步能在一两天内验证所有平台假设**，比先写引擎值得。

---

## 10. 风险清单

| 风险 | 影响 | 应对 |
|---|---|---|
| crate 拆分改坏 Windows 端行为 | 高（现有产品） | 纯剪切、逐文件搬、每步回归、真机逐字对比 |
| `textservice.rs` 的状态机耦合 TSF | 高 | 先画一遍状态图再动手；键路由做成 `fn key(&mut self, Key) -> Action` |
| `zhconv` 换不到等价实现 | 中 | 先用常用字表兜底，繁体准确率略降可接受 |
| APK 体积超预期 | 中 | 核心 so + 词库 ≤ 25 MB；语音模型 228 MB 按需下载，不进安装包 |
| 手机候选条空间不足，功能退化 | 中 | M2 真机实测后重新设计交互，不要照搬桌面 |
| 语音模型下载失败 / 源被墙 | 中 | 主源 ModelScope（实测 4.9 MB/s + Range 续传）；GitHub **实测完全不通，不用**；兜底：重试 → 备用仓库 → 手动选文件 |
| 下载源被第三方作者删库 | 中 | 发布前把 onnx 传进**自己的** ModelScope 仓库，钉死大小 + SHA256，加载前校验 |
| 用户嫌 228 MB 不下 | 中 | 文案写清「只需一次，之后完全离线」；不强推，不下载也能正常打字 |
| 上架合规（国内商店备案） | 中 | 参考星语 `docs/08-合规备案.md` 的备案三件套；输入法类应用审查更严 |

---

## 附：结论重述

- **能复用青简的**：Android IME 的架构、JNI 边界、生命周期坑、隐私开关、候选条渲染思路。
- **必须用自己的**：引擎（双拼 + 音形 + 分词 + 联想），这是飞鹤的差异所在。
- **能用星语的**：sherpa-onnx 离线 ASR 的模型选型与集成样板（记得砍掉声纹）。
- **最大风险**：crate 拆分影响 Windows 现有产品。
- **最省的第一步**：先做一个只出"你好"的 Android IME 验证平台假设，再决定要不要投入 10 周。

---

## 11. 实施进度

| 阶段 | 状态 | 说明 |
|---|---|---|
| **M1 第一步：拆出 `feihe-core`** | **已完成** | 8 个零依赖叶子模块（`shuangpin` / `fuzzy` / `correct` / `number` / `calc` / `brief` / `brief2` / `assist`）搬进 `core/`，62 项单测随迁，总数仍是 308；发布模式 DLL 4285 KB（拆分前 4284 KB）。详见 `handoff.md` |
| **M1 第二步：`config` 搬进 core** | **已完成** | 路径与全屏判断都改成壳注入（`paths::set_resolver` / `config::set_fullscreen_probe`），config 里一行系统调用不剩；313 项（125 + 188）两种配置全绿，DLL 4285/4328 KB 不变 |
| **M1 第三步：数据层整体搬进 core** | **已完成** | `freq` / `next` / `pin` / `demote` / `custom` / `dict` / `dictbin` / `dictenc` / `phrase` 全部入驻。三处开钩子：`core::log`（日志）、`core::clock`（`GetLocalTime` → 可注入）、`dictbin::map_file` 不再真做内存映射。`base.bin` 改由 `core/build.rs` 生成 |
| M1 第四步：`hint` / `fancha` / `zhconv` | **已完成** | `hint`（含 `hint.txt`）整体搬走；`fancha` 开 `set_clipboard_reader`（Windows 剪贴板留在 `src/clipboard.rs`）；`zhconv` 开 `set_converter`，NLS 映射留在 `src/nls.rs`。**317 项测试全绿**（core 206 + 壳 111），DLL 4290 KB 不变，`cargo check --target aarch64-linux-android` 通过。`sogou` **明确不搬**，理由见下 |
| M1 第五步：`textservice` → `Session` 纯状态机 | 待做 | **唯一有风险的一步**，动之前先画状态图 |
| M2 Android 最小可用 | 进行中 | **能打中文，双拼/全拼双模**：双拼模式打 `nihc`、全拼模式打 `nihao`，都是「你好」；`sanguo`→「三国」。26 键 + **3×3 九宫格数字页** + 独立符号页 + 上档（⇧ 一次性大写）+ 组合串 + 上屏/删除，共 **33 项 Kotlin 单测**。**JNI 12 个入口**（`init` / `lookupInput` / `encodePhrase` / `isSyllable` / `coreVersion` / `suggest` / `record` / `config` / `setConfig` / `dataFiles` / `manifest` / `reloadAll`），`.so` 3.6 MiB（3 782 008 字节，含 `base.bin` 2.9 MB + 联想内置表）。**设置页三个入口**（状态条齿轮 / 系统设置 / 桌面图标）：输入方式、双拼方案、繁体、联想、学习、**数据备份（导出/导入 zip）**。**长串自动整句**：`vhsjli`→「张三李」、`nihaoshijie`→「你好世界」（分段规则与桌面共用 core 的 `segment`）。**联想**：上屏后候选条末尾 `→词`，点击上屏并续链；标点/空格/回车/删字/切模式都断开上下文。**真机已验证**（realme RMX5080 / Android 16，2026-11-05）：全链路可用，并揪出内置联想表权重被 `MAX_COUNT` 压平导致顺序倒置的 bug（见 `handoff.md`） |

### 11.1 `sogou` 为什么不搬

它看着像「纯解析」，其实三段都长在 Windows 上：

1. `import_auto` 靠注册表（`HKCU\Software\SogouInput`）找用户的搜狗目录；
2. `.scel` 细胞词库是搜狗私有格式，解析器本身能跨平台，但**Android 上文件从哪来**
   没有答案；
3. 产物写 `user_dict.txt`，那是桌面端「从搜狗导入…」流程的一部分。

搬过去等于在 core 里放一段永远走不到的代码，还得为它维护跨平台测试。真要做
「从文件导入词库」时，把解析部分单独拆成 `wordlist` 模块比现在整包搬干净得多。

> **2026-11-06 改判。** 上面第 2 条「文件从哪来没有答案」被现实推翻了：用户在手机上
> 装完输入法第一件事就是从搜狗词库网站下 `.scel` 再导入。所以按当初说的那样**只拆
> 解析部分**——`core/src/sogou.rs` 现在只有「字节 → 词 → `词<TAB>全拼`」，接口是
> `Job { label, data, kind, prio }` + `encode_jobs` / `encode_one`；注册表发现、目录
> 扫描、写 `user_dict.txt` 全部留在壳里（Windows 是 `src/sogou.rs` 的 `import_auto`
> 等，Android 是 `android/jni/src/sogou_extra.rs`）。见 11.7。

### 11.2 「这串按键是双拼码还是全拼」为什么必须在 core 里判

Windows 端从来不需要回答这个问题：TSF 一个键一个键喂进来，`textservice` 攒的就是码。
Android 的 26 键键盘不一样——用户按的是字母，屏幕上没有「码」的概念，壳必须判断。

第一版图省事，直接在 `FeiheIme.kt` 里写死「按键串一律按全拼，用 `encodePhrase` 转成码」
（当时只有这一条路能出字）。后果就是：**双拼用户打 `nihc` 一个字都出不来**，而
「设置」里那个「全拼模式」开关写不写都没区别——因为壳压根没读它。

现在的规则放在 `core/src/dict.rs` 的 `dict::resolve_code_with` / `lookup_input`：

| 模式（`config.ini` 的 `pinyin_mode`） | 按键怎么解释 |
|---|---|
| 双拼（默认，`0`） | 先原样当码查；**查不到再按全拼解释** |
| 全拼（`1`） | 一律按全拼解释，编不出码就是没有 |

三条设计上的取舍：

1. **判断必须和 Windows 共用一份。** 写在 Kotlin 里就是第二份真值，两端一旦分歧，
   症状是「候选莫名其妙不对」，而没人会想到去比两边的判断规则。
2. **双拼模式要能兜住全拼。** 默认值是双拼（和桌面端同一个字段），如果严格只认双拼，
   新装用户打 `nihao` 会看到一片空白，然后判定「输入法坏了」。所以双拼模式在
   「原样查不到」时退回全拼解释——它实际是「双拼，且认全拼」。
3. **全拼模式不许退回双拼。** 这一条是反过来守的：选了全拼的人要的就是
   `xian` → 「先」而不是「西安」，这时候再让双拼抢答就等于没开关。

代价是双拼模式下少数串有歧义（`xian` 是「西+安」也是「先」），双拼优先——因为
**模式是用户显式选的，按键串不是**。想反过来就在设置里切「全拼」。

单测喂的是**假探针**（`resolve_code_with(input, mode, scheme, probe)`），不查真词库：
`lookup` 的结果进断言这条，`handoff` 里已经记过三次，个人词库一导入当场不成立。

### 11.3 个人词库的单独导入 / 导出

整包备份（11.4）解决「换手机」；但更常见的是「把搜狗词库导进来」和「把手机上的
词带走」。这两件事塞在 zip 里说不通——用户得先知道 zip 是什么。

所以设置页单独一张「个人词库」卡片，三个动作：**导入词库** / **导出词库** /
**编译个人词库**，走 JNI 的 `dictExportNative` / `dictImportNative` /
`dictCompileNative` / `dictSourceNative`（`android/jni/src/dict_extra.rs`）。

- 格式就是 `user_dict.txt` 那一套纯文本（每行「词 + Tab + 全拼」），桌面端的
  「导出词库」给的就是它。**纯文本是刻意的**：能记事本打开、能拷到电脑上用。
- SAF 给的是 `Uri`，引擎只认路径。Kotlin 侧 `dictTransfer(uri, import)` 负责
  「Uri ⇄ cacheDir 临时文件」那一小段，用完就删。**不给 core 传流**：那要么引入
  一次 JNI 回调，要么在 core 里开个只有 Android 用的口子，都比一次拷贝更危险。
- 导出是**先写临时文件、成功了才搬到用户选的位置**，失败不留空文件。
- 编译不是必做的：没编也能查（走文本回退），编了才走映射。`CompileOutcome::NoSource`
  不算失败——给「先导入一份」的人话，不抛 ERR。

### 11.7 搜狗词库（`.scel`）在 Android 上怎么导

Windows 端三件套是「自动发现搜狗目录 / 细胞词库…（选一个 .scel）/ 从搜狗导入」。
Android 上只有中间那件有意义，另外两件没有对应的东西（没有注册表、没有安装目录）。

- **解析在 core**：`feihe_core::sogou`（`Job` + `encode_jobs`）。Windows 和 Android
  共用同一份 `extract_scel` / `extract_bin_*` / 注音逻辑——各写一遍迟早会有一边不认
  新版本的 `.scel`，而那种错是「导入 0 条、界面还说过成功」。
- **Java 侧只传递文件路径**：`sogouImportNative(path)` 读盘、判扩展名、调 core、
  最后 `dict::merge_text` 并进个人词库（会顺手编译 `user.bin`）。
- **返回串带摘要**：`"OK|<多行摘要>|<新增条数>"` 或 `"ERR|<原因>"`。摘要里必带
  「解析 N 条 / 新增 M 条」——只有「导入成功」四个字的话，碰上「格式变了、一条都没
  解析出来」用户完全无从判断。
- **界面必须用对话框，不能用 Toast**：摘要是多行的，Toast 会截断。

### 11.4 数据备份：清单在 core，容器在壳

用户要「导出一份包含全部自有数据的备份」。边界划在**清单**上：

- `feihe_core::backup::DATA_FILES` 说哪些文件算用户数据（设置、个人词库、词频、置顶、
  自造词、联想、降权、自定义短语），`manifest()` 生成带版本和时间的说明，
  `reload_all()` 在导入后把八个模块的内存缓存全部重读。
- 打包/解包在 Kotlin（`java.util.zip`）：core 的依赖政策只允许 `tyme4rs`，zip 要么引库
  要么手写几百行 DEFLATE；而 `java.util.zip` 两行就完事。

两个刻意的决定：

1. **`user.bin` 不在清单里。** 它是 `user_dict.txt` 的编译产物，格式随词库版本变。
   跨版本导入旧 bin，最坏情况是把旧格式当新格式解析、查出一堆乱码词。导入后重编译
   一次只要百来毫秒。同理 `feihe.log` 也不备份——那是诊断日志。
2. **导入 = 恢复到备份，不是合并。** 先清掉所有数据文件再写包里的。合并需要一堆
   「新盖旧还是旧盖新」的规则，而用户说「导入备份」时想要的一定是回到那个时间点。
   代价是包不完整就等于删数据，所以**整个包先解到内存、校验完才动手写盘**，写盘也
   先落 `.importing` 临时文件再改名。

还有一个和「不假装支持」相关的取舍：备份包是**外部输入**，导入端只认 `DATA_FILES`
里的名字（一字不差），`../config.ini` 这类路径天然进不来，单个文件超过 32 MB 直接拒绝
（挡解压炸弹）。

### 11.5 长串自动整句：分段规则从 `textservice` 搬进 core

手机上打 `vhsjli` 之前是空候选条——3 个音节、词库里没有「张三李」这个词，于是什么
都没有。桌面端不是这样：它进分词态，切成 `张三|李` 让人逐段选。手机上没做逐段选
（交互重），但**第一层必须有**：把码切成几段、各段取第一名拼成「张三李」当一个候选。

切分逻辑从 `textservice` 搬到了 `feihe_core::segment`（`syllables` / `split_segments`
/ `plan`），桌面侧只剩转发。这是刻意的：分段规则是「同一个码切出哪些词」的唯一真值，
在 Kotlin 里重写一遍的结果是两端切法不同，而症状是「手机和桌面不一样」，没人查得到原因。

搬的过程中撞出一个真问题：**分段用的「这一段有没有词」不能用 `dict::any`**。它是
**前缀探测**，而词库里有「张三李四」（码 `vhsjlisj`），`any("vhsjli")` 因此为真，
DP 于是认为整串就是一个词、`codes.len() >= 2` 不成立、退回逐音节，整句变成「长三裏」。

修法是加 `dict::any_aligned` / `dict::first_aligned`：**字数必须等于音节数**。
为什么用字数而不是码长字段：码长要走 `collect` 透出来，而那条流水线后面还挂着一简字 /
二简词 / 短语 / 置顶一堆插入，长度数组会失配；字数则是候选自己的属性。
扫描上限只用 12 条也有依据：双拼码长 = 2 × 音节数，前缀匹配要求词码以段码开头，
所以「字数对得上」的恰好是码长最短的那一档，`collect` 的排序把它们放在最前面。

真词库上验过：`vhsjli` → 张三李、`nihaoshijie`（全拼串）→ 你好世界、
`wobuzhidao` → 我不知道、`zhonghuarenmingongheguo` → 中华人民共和国、
`mmap`（编码混英文）→ 整句放弃、什么都不给。耗时 0.1–2 ms，只在「整码查不到」时才跑。

### 11.6 数字页：3×3 九宫格 + 独立符号页

第一版数字页是每行 5 个键（`1 2 3 4 5` / `6 7 8 9 0`），每个键只有屏宽 1/5，
手指容易按偏。改成电话拨号盘那种三列九宫格后，键宽接近 1/3。

三个随之而来的决定：

1. **多一排但不长高。** 字母页 4 排 × 46dp，紧凑页 5 排 × 36dp：
   `4×46 + 3×5 + 12 = 211dp` 对 `5×36 + 4×5 + 12 = 212dp`，差 1dp。
   不然每次切页整块键盘长出 50dp，输入框和候选条都要往上跳一下。
   行高由 `KeyboardPages.rowHeightDp` 给，单测里钉着这个算式。
2. **符号单独一页。** 3 列 × 5 行 = 15 格，装不下「10 数字 + 中文标点 + 功能键」，
   硬塞就得把某排做成 6 个小键，等于把刚争取到的键宽还回去。数字页只留 `，` `。`，
   其余 12 个标点到「符」页。两页**形状完全相同**（5 行、每行总权重 6），
   切换时每个键的位置都不动。
3. **`KeyKind.SYMBOL`（布尔切两页）换成 `KeyKind.PAGE` + `Page` 三态枚举。**
   三个键（`123` / `ABC` / `符`）要去三个地方，布尔值装不下，硬塞只会变成一堆 if。
   `key.code` 直接存目标页名，`FeiheIme` 里只有一个 when。

---

## 12. 本机开发环境（2026-11-05 配好）

### 12.1 装了什么、装在哪

| 组件 | 版本 | 路径 | 来源 |
|---|---|---|---|
| Android SDK | platform **android-34** + build-tools **34.0.0** + platform-tools **34.0.5** | `D:/soft/asdk` | 平台与 platform-tools 本来就在（后者是 AGP 首次构建时自动装的 **37.0.1**）；build-tools 从腾讯镜像补 zip。**注意：platform-tools 于 2026-11-05 被误覆盖成 34.0.5**，元数据已如实改写，原因见 `handoff.md` |
| JDK | **Temurin 17.0.20.1+1** | `D:/soft/jdk17` | 清华 Adoptium 镜像 |
| Gradle | **8.7** | `D:/soft/gradle-8.7` | 腾讯镜像 |
| AGP | **8.5.2** | 由 Gradle 拉 | 阿里云 google 镜像 |
| Kotlin | **2.0.21** | 同上 | 阿里云 public 镜像 |

版本是被工具链钉死的链条，动一个要连带动其余：`compileSdk 34` → 要求 `AGP ≥ 8.1.1`
→ `AGP 8.5` 要求 `Gradle ≥ 8.7` → `AGP 8.x` 要求 **JDK 17**。机器上原有的 JDK 14
和 JDK 8 都不够，AGP 会直接拒绝执行。

### 12.2 三个必须知道的网络事实（实测）

1. **`dl.google.com` 完全不通**（`http=000`，10 秒超时）。所以 `google()` 仓库不能放
   在第一个——所有 `repositories` 块里 **阿里云镜像必须排在最前**：
   `maven.aliyun.com/repository/google` 和 `/public`（实测 0.3 秒响应）。
2. **`services.gradle.org` 用不了**。它返回 307 跳转，Gradle 自己的
   `validateDistributionUrl` 校验会失败（`Test of distribution url … failed`），
   `gradle wrapper` 直接报错。必须显式给镜像地址：
   `gradle wrapper --gradle-distribution-url=https://mirrors.cloud.tencent.com/gradle/gradle-8.7-bin.zip`。
   `gradle-wrapper.properties` 里存的就是这个地址，别改回官方。
3. **`dl.google.com/android/repository` 也不通**，所以 `sdkmanager` 装不了包。
   build-tools 是从 `mirrors.cloud.tencent.com/AndroidSDK/build-tools_r34-windows.zip`
   直接下 zip 解开、目录名改成 `34.0.0` 放进 `build-tools/` 的。以后要补别的 SDK
   包（platform-tools、ndk），也走这条路：下 zip → 改目录名 → 放进对应目录。

### 12.3 构建

**用仓库根的 `build-android.ps1`**，别直接 `gradlew.bat`：本机 PATH 里的 java 是 1.8，
AGP 8.5.2 要 17，直接调会报一句和真实原因无关的错就退出。

```powershell
powershell -ExecutionPolicy Bypass -File build-android.ps1            # release APK
powershell -ExecutionPolicy Bypass -File build-android.ps1 -Debug     # debug APK
powershell -ExecutionPolicy Bypass -File build-android.ps1 -UnitTest  # 只跑 JVM 单测
powershell -ExecutionPolicy Bypass -File build-android.ps1 -ShellCheck # 只查环境
powershell -ExecutionPolicy Bypass -File build-android.ps1 -Install   # 编完直接 adb install
```

脚本把三处路径钉在**进程环境**里（不写注册表、不改用户变量）：

| 变量 | 值 | 为什么 |
|---|---|---|
| `JAVA_HOME` / `PATH` | `D:\soft\jdk17` | AGP 8.x 硬性要求 |
| `ANDROID_HOME` / `ANDROID_SDK_ROOT` | `D:\soft\asdk` | 顺手写 `local.properties` |
| Gradle | `D:\soft\gradle-8.7\bin\gradle.bat` | 不用 wrapper：它要联网下 8.7 发行包 |

另外两件脚本替你做的事：缺 `jniLibs/arm64-v8a/lib*.so` 时调
`tools/fetch_sherpa_libs.py`、缺 `assets/asr/*` 时调 `tools/fetch_asr_model.py`；
Rust 侧自己设 `CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER` 和 `FEIHE_NO_USER_DICT=1`
（**绝不能**把本机个人词库编进要分发的包）。

编完脚本会反查 APK 里三件事，任何一条不过就等于白打包：`lib/arm64-v8a/libfeihe_android.so`
在不在、`assets/asr/model.int8.onnx` 是不是 **STORED**、v2 签名有没有开。

`android/local.properties` 里写 `sdk.dir=D:/soft/asdk`（正斜杠，不转义），这个文件
不进版本库。

### 12.4 骨架已经验到哪一步

目前 `android/` 里是一个**能打字的英文态输入法 + 26 键中文键盘**：
键位表在 `keyboard/KeyboardLayout.kt`（纯数据，6 项单测锁住"26 个字母一个不多一个不少"），
视图在 `keyboard/KeyboardView.kt`（程序化构建，不用 XML 布局），组合串/上屏/删除在
`FeiheIme.kt`。**中文态要等引擎**（`feihe-core` 交叉编译成 `.so` 需要 NDK）。

关键取舍：**键盘不分小鹤/全拼**。小鹤不改键位，改的是"字母怎么解释成声母韵母"，
那是引擎的事。任何"把韵母印到键上"的做法都会让界面和引擎各存一份映射。

已从 APK 里反查确认（不是只看编译成功）：
`BIND_INPUT_METHOD` 权限、`android.view.InputMethod` 意图、`android.view.im`
元数据、`res/xml/method.xml` 都在包里。

模型也已打进 assets 并实测：`assets/asr/model.int8.onnx` 在 APK 里是 **STORED**
（未压缩），说明 `androidResources { noCompress += listOf("onnx","bin","fst") }`
生效了——这一条不做的话，原生层拿不到连续 fd，只能整包解压进内存，79 MB 的模型
冷启动会翻倍。带模型的 debug APK 是 **80.0 MiB**。

### 12.5 Rust 引擎（JNI）的构建

引擎是 `android/jni`（crate 名 `feihe-android`），依赖 `feihe-core`，产出
`libfeihe_android.so`。**不**走 AGP 的 `externalNativeBuild`（那是 CMake 的地盘），
而是在 `app/build.gradle.kts` 里注册一个 `Exec` 任务直接调 cargo，再拷成 jniLibs
期望的目录形状（`rustJniLibs/arm64-v8a/`），挂到 `preBuild` 前面。

补装的东西：

| 组件 | 版本 | 路径 | 来源 |
|---|---|---|---|
| NDK | **r26d（26.3.11579264）** | `D:/soft/asdk/ndk/26.3.11579264` | 腾讯镜像的 `android-ndk-r26d-windows.zip`（635 MB） |
| Rust 目标 | `aarch64-linux-android` | `rustup target add` | **官方源**（清华镜像上这个日期还没有，404），15 分钟 |

#### 三个坑

1. **链接器不能用 `aarch64-linux-android24-clang.cmd`。** 那是个批处理包装，
   Windows 下会把 `-Wl,--version-script=...` 这类参数搞坏，报错还是 **GBK 编码的
   中文**（`此时不应有 --version-script`），从英文 log 里完全看不出是编码问题。
   改用 `clang.exe` + `RUSTFLAGS=-Clink-arg=--target=aarch64-linux-android24`。
2. **`jni` crate 是根 workspace「只允许 tyme4rs」的唯一例外。** 它只编进 Android
   的 `.so`，**不进 Windows 的 DLL**，所以安装包体积一分钱没涨（64 位仍是 4285 KB）。
   手写 JNI 函数表下标也能做到零依赖，但下标写错是一次没有诊断信息的崩溃，而这块
   恰恰是本机最验证不了的。
3. **JNI 符号名 = 包名 + 类名 + 方法名。** Kotlin 侧包装方法叫 `coreVersion()`，
   原生方法必须叫 `coreVersionNative()`——同名没法共存。改包名/类名/改名都要同步
   `android/jni/src/lib.rs`，否则运行期 `UnsatisfiedLinkError`，只有装到真机才发现。

#### 怎么验（不需要真机的那一半）

```bash
# 1. 编出 .so 并看架构
cd android/jni
CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=<ndk>/toolchains/llvm/prebuilt/windows-x86_64/bin/clang.exe \
RUSTFLAGS=-Clink-arg=--target=aarch64-linux-android24 \
cargo build --release --target aarch64-linux-android

# 2. 符号对不对（三个 Java_ 开头的）
<ndk>/toolchains/llvm/prebuilt/windows-x86_64/bin/llvm-nm.exe -D --defined-only \
  target/aarch64-linux-android/release/libfeihe_android.so | grep Java_

# 3. APK 里有没有、是不是被压缩了
python -c "import zipfile;z=zipfile.ZipFile('app/build/outputs/apk/debug/app-debug.apk');\
print([(i.filename,i.compress_type) for i in z.infolist() if i.filename.startswith('lib/')])"
```

真机上验只需要看状态条：它现在会显示 `引擎 0.3.18 (aarch64)`。打 `nihao` 时
应当在同一行看到 `nihao→nihc`——**这是屏幕上第一处真正来自 Rust 的东西**。

### 12.6 编译用签名（`android/keystore/`）

Android 要求 APK 必须签名才能安装。调试包 AGP 会自动用 `~/.android/debug.keystore`
签，但那个 key **每台机器都不一样**；`assembleRelease` 默认更是只产 unsigned 包。
所以项目里放一把固定不变的 key：

| 项 | 值 |
|---|---|
| 密钥库 | `android/keystore/feihe-release.p12`（PKCS12，2.5 KB） |
| 别名 | `feihe` |
| 密码 | `feihe-ime-2026`（store 与 key 同一个） |
| 算法 | RSA 2048 / SHA256withRSA |
| 有效期 | 2026-11-05 → **2056-10-28**（10950 天） |
| 证书 SHA-256 | `5A:E2:C7:06:22:65:45:D9:9D:43:6E:D6:EB:05:50:3C:2F:A6:7C:F8:66:DB:6F:92:C3:C9:E3:5F:DA:4A:6B:29` |
| 证书 SHA-1 | `EB:5F:F0:E5:8C:C3:04:E6:7B:72:A6:B2:77:08:78:6C:4D:8D:78:A4` |

凭据写在 `android/keystore/keystore.properties`，由 `app/build.gradle.kts` 读取。

**两者都进版本库，这是刻意的。** 自签名只解决「同一把 key 打出来的包能互相覆盖」，
不承担任何身份作用；项目只做自用和侧载，不走应用商店。藏起来的结果只会是：换台
机器 clone 下来打不出能覆盖安装的包，还得翻聊天记录找密码。真要上架那天，商店
那份签名必须**重新生成**、不放进这个仓库。

#### 为什么 debug 也用同一把 key

Android 只在**签名相同**时才允许覆盖安装，否则报 `INSTALL_FAILED_UPDATE_INCOMPATIBLE`，
唯一出路是卸载。输入法的数据目录（用户词库、词频、置顶、自造词）跟着应用走，
一卸载就全没了。如果 debug 用调试 key、release 用项目 key，那「先装 debug 试一下、
再换 release」这条最常走的路每次都要卸载重装——统一成一把就没这事。

所以 `buildTypes` 里 debug 和 release 都指向 `signingConfigs.feihe`。

#### 签名方案：v2 + v3，没有 v1

实测 `apksigner verify -v app-release.apk`：`v1: false / v2: true / v3: true`。
v1（jar 签名）是给 Android 6 及更老系统用的，`minSdk = 24` 根本装不上那些版本，
AGP 自己就会跳过——`enableV1Signing = true` 写了也不生效，已显式设 `false` 并注明。

#### 校验

```bash
# 证书是不是同一把（debug 与 release 必须一致）
"$ANDROID_HOME/build-tools/34.0.0/apksigner.bat" verify --print-certs \
  app/build/outputs/apk/release/app-release.apk

# 签名方案
"$ANDROID_HOME/build-tools/34.0.0/apksigner.bat" verify -v app-release.apk
```

`apksigner` 是 `.bat`，运行前 `JAVA_HOME` 要指向 JDK 17。

#### 丢了 key 的后果，以及重新生成

**丢了 = 所有已装的包再也无法覆盖升级**，只能卸载重装，用户词库跟着丢。所以这个
文件值得备份（跟仓库一起走就是最省事的备份）。确实要换时，前提是接受「让所有设备
重装一次」：

```bash
cd android/keystore
"$JDK/bin/keytool" -genkeypair -v \
  -keystore feihe-release.p12 -storetype PKCS12 \
  -alias feihe -keyalg RSA -keysize 2048 -validity 10950 \
  -dname "CN=Feihe Input Method, OU=Feihe, O=Feihe, L=Beijing, ST=Beijing, C=CN" \
  -storepass 'feihe-ime-2026' -keypass 'feihe-ime-2026'
```

换完把新的 SHA-256 指纹更新到本节。**`storeFile` 不要写成绝对路径**——那会让
「文件不存在就退回调试签名」的判断在别人机器上永远为真。

#### 两个会安静出错的点

1. **`.gitattributes` 必须钉 `*.p12 binary`。** 这台机器 `core.autocrlf` 是开的，
   不加这条 git 会把 keystore 当文本加 CRLF，下次 checkout 出来 PKCS12 直接解不开，
   而 keytool 报的是「密码不正确」——离真正原因十万八千里。
2. **缺 key 时 `assembleRelease` 不报错，只产出一个 unsigned APK。** 装不上，而
   错误暴露在 adb 或用户手机上。所以 `app/build.gradle.kts` 末尾给 `assembleRelease`
   挂了 `doFirst` 检查，文件缺失直接抛 `GradleException`。
