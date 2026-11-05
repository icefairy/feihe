# 飞鹤输入法 · Android 平台方案

> 规划日期：2026-11-05
> 参考：`d:\codes\qingjian` 的 `apps/android`（Rust core + JNI + Kotlin 壳的成熟跨平台样例）、
> `d:\codes\xingyu` 的 `m1`（sherpa-onnx 离线 ASR 的验证样板）。
> 目标：把小鹤双拼 / 音形 / 联想做成 Android 输入法（IME），并带上离线语音输入。

---

## 0. 结论先说

**飞鹤 Android 不是「把 TSF 输入法移植过去」，而是「给现有的纯逻辑引擎换一个平台壳」。**

具体分工：

| 层 | 来源 | 说明 |
|---|---|---|
| 引擎（双拼 / 音形 / 词库 / 联想 / 短语 / 模糊音 / 纠错 / 简繁 / 标点） | **飞鹤自己的 Rust 代码**，抽成 `feihe-core` | 算法一行不用重写，但要做一次 crate 拆分 |
| 平台壳骨架（IME 生命周期 / 键盘 / 候选条 / 设置 / 权限 / 剪贴板） | **抄青简 android 的结构** | 它踩过的 Android 坑（键盘重建、无痕输入、输入框类型）都能省掉 |
| 离线语音（ASR） | **星语 m1 的 sherpa-onnx 组合** | SenseVoice + VAD，模型不随包、按需下载 |

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

1. **模型选型已经验证过**：SenseVoiceSmall 的 ONNX 版（int8 后 60–120 MB），CPU 4 核实时（RTF < 1）。这是目前中文离线 ASR 里"体积 / 准确率 / 实时性"最平衡的。
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
| `quick.rs` | 快捷输入 | 依赖 `GetLocalTime`，换 `chrono`-less 的 `std::time` + 时区处理 |
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

**为什么这步最难**：`textservice.rs` 里"组句状态机"和"TSF 调用"是混在一起的（1800 行的 `OnKeyDown`）。拆分时最容易出的错是**顺手改了逻辑**——Windows 端有 308 项单测和一份验证清单，所有行为必须逐条对得上。策略：

1. 先做**纯剪切**：`Session` 只搬状态和决策（`fn key(&mut self, k: Key) -> Action`），不搬任何 `ITf*`。
2. 每个搬出来的函数保留原注释和单测，测试跟着搬。
3. Windows 壳改成"把键事件喂给 Session，执行返回的 Action"。
4. **判定标准**：308 项单测全绿 + 验证清单里 M3.x 全过 ≠ 完成；还要真机打字对比一遍（用同一个词库、同一串输入，输出必须逐字相同）。

工作量粗估：core 拆分 + Windows 回归约 **2–3 周**（`textservice.rs` 占一半）。

---

## 4. 离线语音（ASR）方案

### 4.1 模型选型：沿用星语验证过的组合

| 组件 | 模型 | 体积（int8） | 作用 |
|---|---|---|---|
| VAD | silero VAD | ~1 MB | 判断"说完了没有"，决定什么时候结束录音 |
| ASR | **SenseVoiceSmall**（ONNX） | **~60–120 MB**（星语实测区间） | 中文 / 英 / 粤 / 日 / 韩，CPU 4 核实时 |

选它的理由（星语 `docs/02-技术调研-ASR.md` 已经论证过）：非自回归、CPU 能跑、自带标点与数字规整，比 whisper 系小一个数量级。**不需要**声纹（CAM++ 那 7 MB 是陪孩子的产品才需要的）。

### 4.2 体积是最大的问题，也是最大的决策点

输入法 APK 的理想体积是 **10–20 MB**。一个 100 MB 的模型塞进去，等于把输入法变成一个"语音 App"——用户装输入法时看到 120 MB 会直接放弃。

三个选项：

| 方案 | APK 体积 | 代价 |
|---|---|---|
| A. 模型随包 | 110–130 MB | 简单，但基本不能接受 |
| B. **首次启用语音时下载** | 10–15 MB | 需要联网一次；要处理下载失败、校验、断点 |
| C. **用户手动放入模型文件** | 10–15 MB | 完全离线，符合飞鹤"不联网"的定位；但用户要会找文件 |

**推荐 B + C 并存**：设置页「语音输入」里两个入口——「下载模型（约 100 MB）」和「我已经有模型文件，手动选择」。前者给普通用户，后者给不想联网的人（也顺便服务企业内网场景）。

下载源不用自己的服务器：直接指向 ModelScope / GitHub Release 的官方地址 + 校验 SHA256，和青简拉万象模型是同一个做法。**飞鹤仍然不收集任何数据**，只是发一个 GET。

> 待实测：SenseVoice int8 的确切体积、以及 arm64 单核 RTF。星语给的 60–120 MB 是范围，落地前要量一次。如果超过 100 MB，考虑换成 `sherpa-onnx` 的 paraformer 小模型（中文专用，体积更小，但只支持中文——对输入法其实够用）。

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
- **中间结果**：SenseVoice 是整段识别（非流式），所以第一版没有"边说边出字"。真需要流式就换 `sherpa-onnx` 的 streaming zipformer（体积相近，准确率略低）。
- **取消**：面板上「取消」按钮，或上滑手势。

### 4.5 权限与隐私

- `RECORD_AUDIO`。输入法**不能弹系统权限对话框**（没有 Activity 上下文），必须开一个透明的 Activity 来申请——这是青简踩过的坑（`docs/notes/android.md`「要录音的识别方式先查麦克风权限，没有就开一个透明窗口弹系统权限框」）。
- 语音数据只在内存里，识别完即丢，**不落盘、不上传**。这一点要在设置页写清楚（飞鹤的产品定位就是"不联网"，语音是唯一可能让人怀疑的地方）。
- 密码框 / 无痕模式下**禁用语音入口**。

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
| **M1** | crate 拆分：`feihe-core` + `feihe-win`，`textservice.rs` 拆成 `session.rs`（纯状态机）+ 壳 | **2–3 周** | Windows 端 308 项单测全绿 + 真机逐字对比一致 |
| **M2** | Android 最小可用：IME 骨架 + 26 键键盘 + 候选条 + 双拼输入 + 上屏 + 读配置 | **3–4 周** | 能装、能打字、能选词、能标点 |
| **M3** | 功能补齐：联想 / 短语 / 置顶 / 降权 / 简繁 / 模糊音 / 纠错 / 快捷输入 | **2–3 周** | 与 Windows 端功能对齐（除 Windows 专属项） |
| **M4** | 语音：sherpa-onnx 集成 + 模型下载/手动导入 + 语音面板 | **2 周** | 说话能上屏，离线可用 |
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
| APK 体积超预期 | 中 | 核心 so + 词库 ≤ 10 MB；语音模型按需下载 |
| 手机候选条空间不足，功能退化 | 中 | M2 真机实测后重新设计交互，不要照搬桌面 |
| 语音模型下载源不稳定 / 被墙 | 低 | 双通道：也支持手动放入文件 |
| 上架合规（国内商店备案） | 中 | 参考星语 `docs/08-合规备案.md` 的备案三件套；输入法类应用审查更严 |

---

## 附：结论重述

- **能复用青简的**：Android IME 的架构、JNI 边界、生命周期坑、隐私开关、候选条渲染思路。
- **必须用自己的**：引擎（双拼 + 音形 + 分词 + 联想），这是飞鹤的差异所在。
- **能用星语的**：sherpa-onnx 离线 ASR 的模型选型与集成样板（记得砍掉声纹）。
- **最大风险**：crate 拆分影响 Windows 现有产品。
- **最省的第一步**：先做一个只出"你好"的 Android IME 验证平台假设，再决定要不要投入 10 周。
