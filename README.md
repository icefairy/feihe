# 飞鹤输入法（feihe）

Windows TSF（Text Services Framework）中文输入法。Rust + windows-rs 实现 COM 组件。

**当前版本 0.2.0** ｜ 安装包 1.36 MB ｜ 装完约 6.0 MB ｜ 单字 8006 ｜ 词条 38024 ｜
不联网、不弹窗、无账号 ｜ 115 项单元测试

> 项目主页（GitHub Pages）在 [`index.html`](index.html)，单文件、零依赖，直接当落地页用。
> 启用方式：仓库 `Settings → Pages`，Source 选 `main` 分支根目录即可。

## 里程碑

- **M1（已完成）**：TSF 骨架——注册为系统输入法、收键、组合态、edit session 上屏
- **M2（已完成）**：小鹤双拼引擎 + 内置词库 + 候选窗（纯 Rust，暂不引 librime，原因见下）
- **M2.5（已完成）**：状态浮窗、设置窗（输入模式 / 方案 / 皮肤 / 字形 / 词库管理）、
  符号软键盘、小鹤 + 微软双拼、自然码辅助码 + 小鹤音形、全角标点、简繁切换
- **M3（进行中）**：Shift 中英切换（已完成）／ 候选窗鼠标点击选词（已完成）／
  单键候选数按码长衰减（已完成）／ 一级简码 + 手动置顶（已完成）／
  UILess 模式（已完成）／ 组合串下划线 `ITfDisplayAttributeProvider`（未做）
- **M5**（v2）：语音输入热键 + ASR + AI 修正（星语 SenseVoice + 璇玑网关）

> 关于 librime：它是 C++17 + CMake，与当前的 llvm-mingw/gnullvm 无 MSVC 路线在 C++ 标准库
> ABI 上摩擦较大。双拼本质是「两键 → 一个全拼音节」的查表映射，纯 Rust 几百行即可跑通，
> 等确实需要 rime 的 schema 生态（模糊音、多方案、用户词典）时再引入。

## 和主流输入法比，飞鹤是什么定位

一句话：**一个只有几 MB、不联网、不弹窗、源码全开的双拼输入法**。
不打算取代搜狗的全功能，只服务「只想安安静静打双拼」的人。

体积数据是本机实测（搜狗 16.3.0.3446 装完 134 MB / 81 个文件，用户数据另占 7.8 MB）：

| 维度 | 飞鹤 | 搜狗拼音 | 微软拼音 |
|---|---|---|---|
| 安装包 | **1.36 MB** | 百 MB 量级 | 随系统预装 |
| 装完占用 | **约 6.0 MB**（两份 DLL） | 134 MB + 用户数据 | 随系统 |
| 广告 / 热点推送 | **没有这功能** | 有（可在设置里关） | 无 |
| 账号 / 联网 | **完全不需要** | 登录后云同步 | 微软账户可选 |
| 开源 | **是**（Rust，全文可改） | 否 | 否 |
| 小鹤双拼 | **原生**，不用装方案 | 支持 | 支持 |
| 词库规模 | 3.8 万条 | 数百万级云词库 | 系统词库 |
| 生僻字 | GB2312 + 通用规范汉字表**全收，缺 0** | 全 | 全 |
| 动态词频 | 本地学习 | 云端 + 本地 | 本地 |
| 简繁切换 | Windows NLS，零体积 | 有 | 有 |
| 皮肤 | 3 套内置 | 皮肤商店 | 少量 |
| 模糊音 | **无** | 有 | 有 |
| 语音输入 | **无**（M5 才规划） | 有 | 有 |
| 云同步 / 云词库 | **无** | 有 | 有 |
| 候选窗点击选词 | **有** | 有 | 有 |

**它真正占优的地方，都是「少」带来的：**

- **体积差两个数量级**。词库在构建期预编码成扁平二进制，`include_bytes!` 嵌进 DLL 的
  `.rdata`；PE 加载器 mmap 它，**多个进程共享同一份物理页**。没有运行时解析、没有
  每进程私有堆，所以 3.8 万条词库只值 2.3 MB。
- **没有联网代码**。整个 DLL 的导入表里没有任何网络 API，这不是「隐私承诺」，是
  「没写这个功能」。实测 18 个导入 DLL 全是 `kernel32` / `user32` / `ole32` / CRT 这一层，
  没有 `wininet` / `winhttp` / `ws2_32` / `urlmon`。自己验：

  ```cmd
  objdump -p feihe.dll | findstr /i "DLL Name"
  objdump -p feihe.dll | findstr /i "internet winhttp ws2_32 urlmon"
  ```
- **改一行就能重新编译**。不喜欢候选顺序、想加个快捷输入命令、想换皮肤颜色，
  改 Rust 源码 30 秒出包，不用等官方发版。
- **双拼是一等公民**。不是「全拼输入法附带一个双拼模式」，键位表本身就是规格，
  单元测试拿官方反查结果钉死（`按钮` = `annq` 这种零声母细节）。
- **不自动上屏**。长句一定是「你按空格才落字」，不会替你猜一半先写进文档。

**它明确不如人的地方，也说清楚：**

- **词库差得远**。3.8 万条是「够日常」的量级，不是「什么都有」。专业术语、
  网络新词、人名地名基本查不到，得靠用户词库补（支持导入搜狗个人词库，
  见后文）。
- **没有云**。换电脑不同步，词频学习只在本机。
- **模糊音、语音、手写都还没有**。
- **安装包未签名**，首次运行会撞 SmartScreen。

如果你要的是「功能最全的中文输入法」，用搜狗或微软拼音；如果你要的是
「一个安静、干净、能改的双拼引擎」，飞鹤就是为这个做的。

## 怎么打字（小鹤双拼）

**不是全拼。** 每个音节固定两键：声母键 + 韵母键。

```text
声母键位
zh=v   ch=i   sh=u
其余声母直接用字母本身（b p m f d t n l g k h j q x r z c s y w）

韵母键位（口诀：秋闱软月云梳翅，松拥黛粉更航安，快莺两望奏夏蛙，撇草追鱼滨鸟眠）
q=iu   w=ei   e=e    r=uan   t=ue   y=un   u=u   i=i   o=o/uo  p=ie
a=a    s=ong  d=ai   f=en    g=eng  h=ang  j=an  k=uai  l=iang
z=ou   x=ia   c=ao   v=ui    b=in   n=iao  m=ian
```

规则：
- `zh` / `ch` / `sh` 落在 `v` / `i` / `u`（`zhong`→`vs`、`chi`→`ii`、`shi`→`ui`）。
  注意这三个键与韵母键复用，位置不同不冲突
- 零声母**按韵母长度分三种**（这是最容易记错的一条，官方「小鹤入门」1.1）：

  | 韵母长度 | 规则 | 例 |
  |---|---|---|
  | 1（`a` `o` `e`） | 重复该字母 | 啊 `aa`、哦 `oo`、额 `ee` |
  | 2（`ai` `ei` `ao` `ou` `an` `en` `er`） | 零声母 + 韵母**末**字母，**表现同全拼** | 爱 `ai`、安 `an`、恩 `en`、欧 `ou` |
  | 3（`ang` `eng`） | 零声母 + 韵母所在键 | 昂 `ah`、鞥 `eg` |

  所以「按钮」是 `annq`（`an` + `nq`），不是 `ajnq`。
- `ü` 写作 `v`（`lv`→`lv`、`xue`→`xt`）

例子：

| 想打 | 全拼 | 实际敲 |
|---|---|---|
| 你好 | ni hao | `nihc` |
| 中国 | zhong guo | `vsgo` |
| 我们 | wo men | `womf` |
| 世界 | shi jie | `uijp` |
| 是 | shi | `ui` |
| 水 | shui | `uv` |
| 的 | de | `de` |
| 按钮 | an niu | `annq` |
| 安全 | an quan | `anqr` |
| 爱情 | ai qing | `aiqk` |
| 欧洲 | ou zhou | `ouvz` |
| 肮脏 | ang zang | `ahzh` |

操作：**空格**选第一个候选，**回车**把已敲的编码原样上屏（`iuliqi` → `iuliqi`，
不选候选），数字键 `1-9` 选对应候选，`←/→` 页内移动，`-` `=` 或 `↑` `↓` 翻页
（`-`/`=` 是搜狗、微软拼音的通行键位；组合态下这两个键归翻页，想打 `－` `＝` 先上屏）。
退格删一个键（分词模式下一次退一个音节，见下节），`Esc` 取消。`Ctrl` / `Alt` / `Win` 组合键一律放行不拦截
（两个例外：`Ctrl+数字` 置顶、`Ctrl+Shift+U` 切 UILess，见下）。

### 一级简码

26 个键各配一个「这个键底下最常用的字」，敲一个键它就是第一候选，空格直接上屏：

```text
q 去   w 我   e 二   r 人   t 他   y 一   u 是   i 出   o 哦   p 平
a 啊   s 三   d 的   f 非   g 个   h 和   j 就   k 可   l 了
z 在   x 小   c 才   v 这   b 不   n 你   m 没
```

口诀：去我二人他，一是出哦平；啊三的非个，和就可了；在小才这，不你没。

`u`=是、`v`=这、`i`=出 看着奇怪，其实是一致的：小鹤双拼里 `u`/`i`/`v` 兼 `sh`/`ch`/`zh`，
所以「是/出/这」的码分别以 `u`/`i`/`v` 开头。

只对**单键**生效，敲第二个键就回到正常排序。一简字排在动态调频前面——这是打字的
肌肉记忆，不该被偶发学习顶掉；想换就用下面的置顶。

### 手动置顶

`Ctrl` + `数字`：把这个候选钉到当前编码上并上屏，以后这个码永远先出它。
对已经置顶的那个词再按一次同样的组合就是取消。

优先级：**手动置顶 > 一级简码 > 动态词频 > 静态词频**。置顶记录在
`%APPDATA%\Feihe\pinned.txt`（`编码<TAB>词`），可手改、可整删；设置窗能看能清空。

> 置顶**不受 `limit` 截断限制**：词排到第 90 名以外也能被钉上来，这是它和
> 动态词频最本质的区别（后者只能重排「已经在候选列表里」的词）。


**鼠标也能选词**：直接点候选窗里的那一项就上屏。窗口不抢输入框焦点，点完继续打字
仍然进组合态；按下后拖到窗外再松开算取消，不会误上屏。候选数量按码长衰减——
单键 18 个（2 页）、两键 45 个，三键以上才满 90。

**单击 Shift** 切中英文（按住期间按过别的键就不算单击，`Shift+A` 照常是大写 A）。
切到英文后所有按键原样进应用，候选窗不弹。

**不会自动上屏。** 只有显式动作（空格 / 数字 / 回车 / 标点）才会把字写进文档。
音节敲完但整串查不到候选时，输入法不会替你挑一个先放着——它切成多段挂起来，
让你一段一段选（见下节）。想结束输入就按空格或回车。

> 早期版本做过「边打边上屏」：整串没候选时把左边能匹配的部分直接上屏、剩下的开新一轮。
> 实际用起来是灾难——打 `viuiq` 想输入「指示器」，屏幕上凭空出现「只是」，
> 后面的 `qi` 又变成一个新的单字组合。已删除。

### 长句：智能分隔 + 分词选择

打得多、整串又查不到词的时候（`nihaoshijie` 这种），飞鹤会先把编码切成音节，
再按「每段都能出候选」切成若干段，进入**分词模式**：

```text
组合串显示：[你好]世界      ← [] 里是当前段
←/→   移到上一段 / 下一段
1-9   给当前段换词（候选窗显示的是这一段的候选）
空格  确认当前段并跳到下一段（最后一段则整体上屏）
回车  一次性整体上屏
退格  拆开当前段（词 → 单字）；全是单字了才退掉最后一个音节
继续敲编码键 → 延长组合串并重新分段，已选过的段会保留
```

**退格拆词。** 自动分段的依据是「词库里能匹配上的最长段」，所以 `tsirll` 会切成
`[通传]两`。如果你要的是「通」+「传」+「两」，按一次退格把 `通传` 拆成两个字——
候选窗立刻变成「通」的字候选，选完自动跳到「传」。**这一步不丢编码**，再按一次退格
才真的删掉末尾音节（`tsirll` → `tsir`）。

光标停在单字上时，拆的是最后一个还能拆的段：退格的方向是从右往左，你想改的通常是
刚打出来的那个词。全拆成单字之后，再退就是退音节，回到「整串有候选」的普通编码态。

要一开始就不让它乱组词，也可以直接敲分隔符 `'` 指定边界（见下节）。

**继续敲字母不会上屏**，只会把编码接长、重新分段。所以长句可以一口气打完再逐段挑。

全拼和双拼都支持。全拼的音节边界不定长（`ni` 两字符、`hao` 三字符、`zhuang` 六字符），
用 DP + 音节合法性校验来切，长音节优先（`xian` 不会切成 `xi`+`an`）。

### 手动分隔

自动切分不理想时，可以用 `;` 强制断一刀：

| 模式 | 分隔键 | 原因 |
| --- | --- | --- |
| 全拼 / 小鹤双拼 | `;` | 这两种编码里没有分号 |
| 微软双拼 | `'` | 分号被 `ing` 韵母占用（`;` = ing），只能换成单引号 |

例子：`xi'an` 强制切成 `xi`+`an` 而不是 `xian`；`ni'hao` 断在 `ni` 后面。
分隔符只是打字辅助，**不会进文档**（回车原样上屏时会去掉）。
位置不合理的分隔符（开头、连续两个、双拼切在半截音节上）会被直接吞掉，不打扰输入。

`Tab` 进入**辅助码**筛选（组合串会显示成 `de b` 这种形式），再按 `Tab` 退出。
辅助码方案在设置里选：自然码辅助码（1 键，取部首声母，如 的→白→`b`）或
小鹤音形（2 键，首末部件，如 打→扌+丁→`sd`）。关掉时 `Tab` 原样还给应用。

## 快捷输入（日期 / 时间 / 农历 / 干支 / 节气）

先敲 `/`（斜杠，`?` 那个键），后面接命令码。**只打首字母也行**——八个命令的
首字母互不相同，`/r` 就等于 `/rq`。

| 输入 | 得到 |
|---|---|
| `/rq` | `2026年10月22日` / `2026-10-22` / `2026/10/22` / `2026.10.22` / `10月22日` / `22日` |
| `/sj` | `17:40` / `17:40:06` / `下午5点40分` / `下午5:40` |
| `/xq` | `星期四` / `周四` |
| `/dt` | `2026-10-22 17:40:07` / `2026-10-22 17:40` / `2026年10月22日 17:40` / `… 星期四` |
| `/ts` | Unix 秒 / Unix 毫秒 |
| `/nl` | 农历：`农历丙午年九月十四` / `丙午年九月十四` / `九月十四` |
| `/gz` | 干支四柱：`丙午年 戊戌月 庚午日 壬午时` / `丙午 戊戌 庚午 壬午` / 三柱（不带时） |
| `/jq` | 下一个节气：`2026年11月7日 立冬` / `11月7日 立冬` / `立冬` |

只敲一个 `/` 会列出全部命令的**当前值预览**（按空格拿日期，按数字选别的），不用记码。

**农历 / 干支 / 节气统一用 [Tyme](https://6tail.cn/tyme.html)**（`tyme4rs`，MIT）：
节气算法引自寿星天文历，农历、干支、节气共用同一套儒略日，三者不会互相打架。
自己写这套等于重造万年历，边界（闰月、立春换年、子时换日）几乎必错。

两个容易误解的点：

- `/nl` 的干支年**按春节换年**（农历年），`/gz` 的年柱**按立春换**（四柱年）。
  春节和立春之间那几天，两者给的年不一样——这是对的，本来就是两套历法。
- `/jq` 给的是「最近的下一个」节气，精确到日；今天正好是节气当天就给今天。

> **为什么不像搜狗那样直接打 `rq`？** 搜狗是全拼，`rq`、`sj` 不会撞车。飞鹤是双拼，
> 两字母恰好就是一个音节：`sj` = `s`+`an` = `san`（三），`xq` = `x`+`iu` = `xiu`（休）。
> 照搬的话一半命令会被词库吃掉，行为不可预测。所以统一加引导符。
>
> **引导符为什么是 `/`？** 顿号只留 `\` 一个键（搜狗习惯）就够了，`/` 位置好记好按，
> 拿来当功能入口更划算。中间一度搬到反引号（理由是「`/` 和 `\` 都要出顿号」），
> 但反引号太偏、记不住，最后还是换回 `/`。代价是中文态下 `/` 不再出 `、`——
> 想打半角斜杠，敲 `/` 再回车（只敲了引导符本身时，回车原样上屏 `/` 或 `／`）。

## 标点

开「全角标点」（新装默认开）后，中文态下这些键直接出中文标点：

| 键 | 得到 |
|---|---|
| `,` `.` | `，` `。` |
| `\` | `、`（顿号只留这一个键，搜狗习惯） |
| `;` | `；` |
| `[` `]` | `【` `】` |
| `'` | `’` |
| `-` `=` | `－` `＝` |
| `/` | 不是标点，是快捷输入引导符 |

Shift 组合一律交给应用自己算，所以 `Shift+\` 出 `|`、`Shift+/` 出 `?`、`Shift+,` 出 `<`。

组合串还没上屏时按标点键也走同一套规则：先把候选上屏、紧跟着粘上全角标点，
**不会**出现「，,」两个字符。

## 状态浮窗与设置

屏幕右下角（可拖动，位置会记住）有一条小横条，四个按钮：

| 按钮 | 作用 |
|---|---|
| 中 / 英 | 切换中英文（等价于单击 Shift）。开了繁体时这里显示「繁」 |
| 全 / 半 | 全角标点开关（`,。\、；【】’－＝` 这一排） |
| 符号 | 弹出 9×5 特殊符号面板，点一下插到光标处 |
| 设置 | 打开设置窗 |

> 皮肤和「全拼 / 双拼」原来也在这条浮窗上，后来挪进设置窗了——它们改一次就长期不变，
> 占着浮窗宽度不划算。浮窗只留随时要按的开关。

设置窗：输入模式（双拼 / 全拼）、双拼方案（小鹤 / 微软）、辅助码方案（关闭 / 自然码 /
小鹤音形）、皮肤（浅色 / 深色 / 护眼）、字形（简体 / 繁体）、全角标点、
浮窗是否跟随光标、**UILess 模式**、动态调频（含「重置」）、**置顶（含「清空」）**、
词库统计 + 导入 / 导出 / 编译。

全拼模式是给访客的应急通道：切到全拼就能直接敲 `nihao` 出「你好」，不用学双拼。
词库走另一套「全拼去空格」索引，第一次切过去时建索引约 60ms，之后复用。
早期它不写盘（怕借电脑的人忘了切回来），现在放进设置窗就必须持久化——
设置项改了重启就失效太费解。

**繁体**是逐字映射（`设置` → `設置`），走 Windows 自带的 NLS 表
（`LCMapStringEx` + `LCMAP_TRADITIONAL_CHINESE`），不带外部字表、不增加安装包体积。
候选窗、组合串、上屏文本三处一起转，看到什么就打出什么。转换失败时原样输出简体，
绝不吞字。

**动态词频**：你实际选了哪个词，下次它就往前排。静态词频来自 rime-ice 的通用语料，
解决不了「我的高频词」——医生打「诊断」、程序员打「函数」，在通用语料里都排不到前面。
学习记录存在 `%APPDATA%\Feihe\freq.txt`（纯文本 `词<TAB>次数`，删掉即重置，
设置窗里也有「重置」按钮）。只学「从候选里选中的词」，回车原样上屏、快捷输入、
软键盘符号都不学。排序是**稳定排序**：没学过的词相对顺序完全不变，
学过的按次数沉浮——不会因为开了这个功能把候选顺序搞乱。

多个宿主进程（记事本 / Word / 浏览器）各有一份内存表，所以写盘前会先把磁盘内容
合并进来（同词取较大值，不相加），切输入法时再按文件 mtime 拉取别的进程学到的词。
否则会出现「在 Word 里学到的词，切到浏览器里又沉回去」这种时灵时不灵的症状。

配置存在 `%APPDATA%\Feihe\config.ini`，改完立即生效（同一进程内即时，其它已打开
的应用要重新切一下输入法）。文件里带 `config_ver`；老配置（v1 把全角标点默认成关）
会一次性迁移成全角标点开——之后再手动关掉不会被改回去。

### UILess 模式（全屏游戏 / 远程桌面）

打开后**一个自绘窗口都不弹**：候选窗、状态浮窗全部消失，只留宿主自己的组合串。
候选改塞进组合串里显示：

```text
nihc  1.你好 2.拟好 3.你号 …
```

拦截点放在 `candidate::show` 和 `statusbar::show`——它们是所有显示路径的唯一出口，
拦在上层的话将来加个调用点就会漏。开关本身是原子量（`config::uileless()`），
因为每次按键都要问一遍。

两个必须处理的问题：

1. **看不见候选就等于盲打。** 双拼同码字多（`de` 有几十个），只给「空格出第一候选」
   是不够的，所以候选要拼进组合串。组合串是**宿主画在文档里的**，不是我们弹的窗口，
   在全屏程序里通常仍能显示。
2. **浮窗是进设置窗的唯一入口。** 藏了浮窗就等于把用户锁在外面，所以必须有键盘出口：
   `Ctrl+Shift+U` 切回。这个键要在「修饰键一律交还应用」那条之前拦。

做不到的部分：完全没接 TSF 的程序（很多独占全屏 DirectX 游戏）里连组合串都没有，
飞鹤在里面一个字也打不出来。那需要外挂式键盘钩子 + `SendInput` 的独立进程，
要常驻监听全部按键，不在本项目范围内。


## 构建（Windows 机）

本机环境走 **llvm-mingw** 路线，**无需 MSVC**（VS 未装 C++ 工具集）。
默认**同时产出 64 位和 32 位两份 DLL**。

前置依赖：
- Rust（rustup）
- [llvm-mingw](https://github.com/mstorsjo/llvm-mingw/releases) 解压到 `C:\llvm-mingw`（需含顶层 `bin\`，即 `x86_64-w64-mingw32-gcc.exe`；`i686-w64-mingw32-*` 同一份压缩包里就有）
- `rustup target add x86_64-pc-windows-gnullvm`
- 32 位额外需要：`rustup target add i686-pc-windows-gnullvm`
  （国内直连不通时用镜像：`$env:RUSTUP_DIST_SERVER='https://mirrors.ustc.edu.cn/rust-static'`
  ——清华/阿里镜像不保留历史日期目录，会 404）

一键构建（推荐）：

```powershell
powershell -ExecutionPolicy Bypass -File build.ps1            # 64 + 32 位
powershell -ExecutionPolicy Bypass -File build.ps1 -Arch x64  # 只编 64 位
powershell -ExecutionPolicy Bypass -File build.ps1 -Arch x86  # 只编 32 位
# 产物：
#   target\x86_64-pc-windows-gnullvm\release\feihe.dll   (Machine=0x8664, 64 位)
#   target\i686-pc-windows-gnullvm\release\feihe.dll     (Machine=0x014c, 32 位)
```

`build.ps1` 会自动：定位 llvm-mingw → 为每个架构生成 `-lgcc`/`-lgcc_eh` 兼容库
（compiler-rt + libunwind）→ 禁用 `libunwind.dll.a`（强制静态链接，否则 ctfmon 找不到该 DLL
导致输入法加载失败）→ 设置 PATH → `cargo build` → **校验 PE 头机器类型** →
用对应位数的测试程序跑一遍 `LoadLibrary` + `DllGetClassObject` + `CreateInstance`。

> 背景：Rust 的 `*-gnu` target 硬编码链接 `-lgcc`/`-lgcc_eh`，而 llvm-mingw 用 compiler-rt/libunwind，故需 shim 库。
> `rust-toolchain.toml` / `.cargo/config.toml` 已固定 toolchain 与 target，无需手动指定。

运行时依赖只有一个第三方 crate：[`tyme4rs`](https://github.com/6tail/tyme4rs)（农历 / 干支 / 节气，
MIT）。其余全部是 `windows` / `windows-core` 和标准库——没有 `chrono`、没有 `serde`、没有网络栈。
`tyme4rs` 自己依赖 `regex`（解析中文名称用），那是 DLL 里唯一一块「不必要但甩不掉」的体积。

### 为什么要两份 DLL

in-proc COM 的位数**必须**和宿主进程一致，64 位 DLL 塞不进 32 位进程（反之亦然）。
所以同一个 CLSID 背后挂了两个文件：

| 宿主 | 加载 | 注册表位置 |
| --- | --- | --- |
| 资源管理器 / TextInputHost / 64 位 Office | `feihe.dll` | `HKLM\SOFTWARE\Classes\CLSID\{...}` |
| 32 位 Office、IE 插件、老的 32 位应用 | `feihe32.dll` | `HKLM\SOFTWARE\Classes\Wow6432Node\CLSID\{...}` |

目录本身取决于安装方式：`register.cmd` 用 `%ProgramData%\Feihe\`，
NSIS 安装包用 `%ProgramFiles%\Feihe\`（并会在安装时尽力清掉老的 ProgramData 副本）。

`installer\register.cmd` 两份都会部署并分别注册；`unregister.cmd` 两处都清。
如果只编了 64 位，安装脚本会给出警告并只注册 64 位那份。

### 32 位构建

`i686-pc-windows-gnullvm` 是官方 target，和 64 位同一条路线，`build.ps1` 里就是多跑一次
`cargo build --target i686-pc-windows-gnullvm`，没有额外垫片。

> 别用 `i686-pc-windows-gnu`。它也能编出来，但会多带 `rsbegin.o`，引用 GCC 老式的
> `___register_frame_info` / `___deregister_frame_info`；llvm-mingw 用 libunwind + SEH，
> 没有这两个符号，链接直接失败，得自己写空实现垫上。既然有 gnullvm，没必要绕这一圈。
> （本项目早期误判「官方没有 i686 gnullvm」，走过一次弯路，已切回 gnullvm。）

### 32 位 TIP 注册表要写两遍

`HKLM\SOFTWARE\Microsoft\CTF\TIP` 是**被 WOW64 重定向**的（CTF 下只有 `SystemShared` 是共享键），
32 位进程读的是 `HKLM\SOFTWARE\Wow6432Node\Microsoft\CTF\TIP`。
只写 64 位那份的话，32 位宿主里飞鹤根本不出现在输入法列表。
`installer\install_tip.ps1` 用同一个函数把 CLSID + Category + LanguageProfile 写进两个视图。


## 测试

```bash
# 单元测试（双拼编码 / 词库查询 / 简繁 / 动态词频 / 分词 / 简码 / 置顶，共 115 项）
cargo test --release          # 需先让 PATH 含 llvm-mingw 的 bin（build.ps1 里有）

# COM 骨架冒烟测试：加载 DLL → DllGetClassObject → CreateInstance → 引用计数归零
python tools/smoke_com.py
```

冒烟测试覆盖：`DllCanUnloadNow` 初始 S_OK、类厂可创建、文本服务对象可实例化、
对象存活时 S_FALSE、`LockServer` 生效、全部释放后回到 S_OK。任何一步失败都说明
注册或导出有问题，不用去 GUI 里瞎试。

原生 MSVC 路线（若日后补装 VS Build Tools C++ 工作负载）：

```powershell
rustup target add x86_64-pc-windows-msvc
cargo build --target x86_64-pc-windows-msvc --release
```

## 安装 / 卸载

**需要管理员**（要写 HKLM 的 COM 注册与 CTF TIP 定义）：

```cmd
cd installer
register.cmd      :: 部署 DLL 到 C:\ProgramData\Feihe\ + 注册 COM/TIP + 加入当前用户键盘列表
unregister.cmd    :: 全部移除
```

安装后到 `设置 → 时间和语言 → 语言和区域 → 中文(简体) → 语言选项 → 键盘` 里应出现「飞鹤拼音」，
用 `Win+Space` 或 `Ctrl+Shift` 切换。**若列表未刷新，注销重登一次**（输入法列表由 shell 缓存）。

### 关键实现要点（踩过的坑）

- DLL 必须部署到**稳定路径**（`%ProgramData%\Feihe\`）。早先直接注册 `target\...\release\feihe.dll`，
  一次 `cargo clean` 输入法就变砖。
- 用户键盘列表的真实位置是 `HKCU\Control Panel\International\User Profile\zh-Hans-CN`，
  值是 `0804:{CLSID}{PROFILE}` = 顺序号(DWORD)，**不是** `SortOrder`。两处都要有。
- `input.dll` **没有** `UninstallLayoutOrTip` 导出；卸载要用 `InstallLayoutOrTip(psz, ILOT_UNINSTALL=0x1)`。
  `InstallLayoutOrTipUserReg` 虽然存在但实测返回 FALSE，不要用。
- 历史遗留的 `HKCU\SOFTWARE\Microsoft\CTF\TIP\{CLSID}\LanguageProfile\...\Enable = 0` 会让条目
  即使注册成功也不可见，安装前要先清掉。

## 打安装包（NSIS）

开发期用 `register.cmd` 就够；要发给别人则用安装包，一键装到 `C:\Program Files\Feihe`：

```powershell
powershell -ExecutionPolicy Bypass -File installer\build_setup.ps1
# 产物：dist\Feihe-Setup-0.2.0.exe
```

`build_setup.ps1` 做四件事：

1. `build.ps1 -NoUserDict -Arch both` —— **发布模式**重编两份 DLL
2. 校验 `target\build_mode.txt`，确认不是拿带个人词库的构建去打包
3. 编译 `installer\feihe.nsi`（`makensis` 默认找 `D:\soft\nsis`，可用 `-NsisDir` 改）
4. 报告产物路径与体积

只改打包不想重编时加 `-SkipBuild`。

### 为什么发布模式要单独一个开关

`src/user_dict.txt` 是从本机搜狗导出的**个人打字记录（约 5 万条）**，普通构建会并进
`base.bin`。这份东西跟着 DLL 发出去就抠不回来了。所以 `build.rs` 认环境变量
`FEIHE_NO_USER_DICT=1` 时跳过它：

| 构建方式 | base.bin | 64 位 DLL |
| --- | --- | --- |
| 普通 | 86749 条 / 6.5 MB | 7.0 MB |
| `-NoUserDict`（发布） | 38024 条 / 2.3 MB | 3.0 MB |

`build_setup.ps1` 还会读 `src\user_dict.txt` 的条数做二次兜底 —— 万一有人用普通构建的
产物打包，它直接报错退出；确实想带个人词库得显式加 `-AllowPersonalDict`。

### 安装包里为什么没有 Python

`tools\*.py`（`import_sogou.py`、`gen_assist.py`、`gen_dict.py`、`smoke_com.py`）全是**开发期**工具：
导词库、生成辅助码、跑冒烟测试。运行时一个都不需要，所以不随安装包分发，
也就没有内置 Python 的必要。安装包实际只装 4 个文件：

```
feihe.dll          64 位
feihe32.dll        32 位
install_tip.ps1    注册脚本（卸载要用，出问题也能手工重跑）
uninstall_tip.ps1  注销脚本
```

`register.cmd` / `unregister.cmd` 是开发期的等价物，不进安装包。

### 打包踩过的坑

- **`.nsi` 必须带 UTF-8 BOM**。NSIS 只在文件开头有 BOM 时才按 UTF-8 解析，否则按系统
  ACP（简中 = GBK）读。脚本里有中文注释，丢了 BOM 就报
  `Bad text encoding: feihe.nsi:1` / `Error in script "feihe.nsi" on line 1` ——
  这个报错完全指不到真因，极难查。`build_setup.ps1` 现在每次编译前自动补 BOM。
- **DLL 必须用 `File` 内嵌**，不能用 `CopyFiles`。`CopyFiles` 是**运行时**拷贝，
  源路径写的是构建机上的 `target\...`，用户机器上根本没有，打出来的包只有 192 KB 空壳。
  现在的做法是先 `File` 解到 `$PLUGINSDIR\ime`，再从那儿 `CopyFiles` 到 `$INSTDIR`。
- **覆盖被占用的 DLL 只能「改名再拷」**。ctfmon / explorer 常常已经把旧 DLL 映射进进程，
  直接 `CopyFiles` 覆盖会失败；Windows 允许**重命名**已映射的文件，所以改名成 `.old`
  再拷新的，`.old` 排到重启删（`Delete /REBOOTOK`）。
- **NSIS 是 32 位程序**，它启动的 PowerShell 也是 32 位。32 位进程里写
  `Wow6432Node\...` 会二次重定向成 `Wow6432Node\Wow6432Node\...`，所以
  `install_tip.ps1` 开头会检测到 32 位实例就切 `Sysnative` 重跑。
- NSIS 3.13 **没有内置 `$PROGRAMDATA` 常量**，要 `ReadEnvStr $R0 "ProgramData"`。
- `makensis` 的 `/D` 选项：`/` 和 `D` 之间不能有空格，值不要带尾反斜杠。
  在 bash 里传 `-DOUTDIR=D:\...` 会被 shell 吃成 `D://...`，要么走 PowerShell，
  要么用正斜杠。

安装包**未签名**，首次运行会撞 SmartScreen「已保护你的电脑」提示，点「更多信息 → 仍要运行」。

## 目录

```
index.html            项目主页（GitHub Pages 用，单文件零依赖，内嵌全部 CSS）
src/lib.rs            DLL 入口（DllGetClassObject / DllCanUnloadNow）
src/clsid.rs          固定 GUID
src/classfactory.rs   IClassFactory
src/dllcount.rs       COM 对象计数（DllCanUnloadNow 用）
src/textservice.rs    TSF 核心（Activate / 按键 sink / 组合态 / edit session / 候选调度）
src/shuangpin.rs      小鹤双拼编码（全拼→两键码，带单测当规格）
src/dict.rs           词库查询（静态二进制 + 运行时覆盖层，二分前缀扫描）
src/dictbin.rs        词库二进制的零解析读取器（段偏移 + 索引表 + 字节池）
src/dictenc.rs        FHDB 编码器（build.rs 与运行期各 #[path] 引入一次，只此一份）
src/dict.txt          基础词库（38573 条 / 626KB，gen_dict.py 生成，勿手改）
src/extra_dict.txt    常用词补充表（76 条，手工维护，只放基础词库里缺的词）
src/user_dict.txt     用户词库（import_sogou.py 生成，可反复重跑；未导入时为空文件）
build.rs              生成 base.bin（预编码三套码）+ 编译图标资源
src/candidate.rs      候选窗（自绘 WS_POPUP + GDI，不抢焦点，支持鼠标点选）
src/statusbar.rs      状态浮窗（中/英、全/半、符号、设置；可拖动 + 前台判定）
src/softkbd.rs        特殊符号软键盘（9×5 自绘网格）
src/settings.rs       设置窗（原生控件：输入模式 / 方案 / 皮肤 / 字形 / 全角 / UILess / 调频 / 置顶 / 词库）
src/freq.rs           动态词频（选词计数 → 稳定重排候选，freq.txt 落盘）
src/brief.rs          一级简码表（26 键一简字，单键查询时提到第一）
src/pin.rs            手动置顶（编码 → 词，pinned.txt 落盘，压过一切默认排序）
src/zhconv.rs         简繁转换（LCMapStringEx + LCMAP_TRADITIONAL_CHINESE）
src/theme.rs          皮肤调色板（浅色 / 深色 / 护眼）
src/font.rs           自绘窗口共用字体 + 填充/描边/文字居中
src/insert.rs         任意窗口往文档写文本、读写中英开关（同步 edit session）
src/config.rs         配置读写（%APPDATA%\Feihe\config.ini）
src/assist.rs         辅助码查表（自然码 / 小鹤音形）
src/assist.txt        辅助码表（3739 字，gen_assist.py 生成，勿手改）
tools/gen_dict.py     基础词库生成器（拉雾凇拼音 rime-ice，按词频取前 N 条）
tools/import_sogou.py 搜狗词库导入器（解析本机搜狗用户词库 → user_dict.txt）
tools/gen_assist.py   辅助码表生成器（Unihan 部首 + cjkvi-ids 部件分解）
tools/smoke_com.py    COM 骨架冒烟测试（ctypes 直调 DllGetClassObject）
shim/loadtest.c       位数无关的 DLL 冒烟测试（LoadLibrary + DllGetClassObject + CreateInstance）
installer/install_tip.ps1    注册：COM + HKLM TIP + 用户键盘列表（UTF-8 BOM，含中文名）
installer/uninstall_tip.ps1  卸载：ILOT_UNINSTALL + 注册表清理（纯 ASCII）
installer/register.cmd       安装入口（管理员，部署 64 + 32 位 DLL）
installer/unregister.cmd     卸载入口（管理员）
build.ps1             一键构建（llvm-mingw，默认 64 + 32 位；-Arch / -Debug / -SkipTest）
check_env.ps1         环境自检（llvm-mingw / rust target / NSIS 是否就位）
docs/使用帮助.md       用户手册（随安装包装进安装目录，UTF-8 BOM 便于记事本打开）
dist/                 安装包产物（已 gitignore）
rust-toolchain.toml   固定 gnu toolchain + gnullvm target
.cargo/config.toml    固定构建 target 与 linker（含 i686 的 linker）
```

## 词库

数据源是[雾凇拼音 rime-ice](https://github.com/iDvel/rime-ice)（带真实整数词频）：

- `cn_dicts/8105.dict.yaml` 单字，**全收**（7896 个可编码，不按词频截断）
- `cn_dicts/base.dict.yaml` 词组，取词频前 30000
- **GB2312 兜底补字**（见下）：8105 表没覆盖到的 GB2312 汉字，从
  [pinyin-data](https://github.com/mozillazg/pinyin-data) 取音补进来

最终单字 **8006 个**（去重后），三条覆盖线全绿：

| 字表 | 字数 | 词库覆盖 | 缺 |
|---|---|---|---|
| 《通用规范汉字表》 | 8105 | 7896 可编码的全部 | 0 |
| GB2312（一级 3755 + 二级 3008） | 6763 | 6763 | **0** |
| 合计去重 | — | **8006** | — |

```bash
python tools/gen_dict.py               # 下载（缓存 _ice*.yaml）并生成
python tools/gen_dict.py --no-download # 只用缓存
```

输出格式是 `词<TAB>全拼`（音节空格分隔），按词频降序。**不存双拼码**——存码等于把方案
焊死在数据里，切小鹤/微软就得重新生成整个词库，导入导出也只能同方案流通。

> **单字为什么全收而不按词频截断？**
> 早期这里取词频前 3500 个单字，结果同事姓「仝」，「仝」在 8105 字表里排 5402 名
> （词频 796，而第 3500 名的下限是 11418）——被直接截掉，翻页也翻不到，表现成
> 「这个字根本打不出来」。生僻字在候选列表里只是尾部多一行，缺了却是硬伤，
> 姓名、地名、古籍用字都吃这个亏。现在 8006 个可编码单字全收，回归测试
> `rare_name_chars_are_reachable` 钉死。

> **为什么还要补 GB2312？**
> 《通用规范汉字表》并不包含全部 GB2312 二级字——实测 GB2312 的 6763 个汉字里有
> **110 个**不在 8105 表里：`嬲 欷 榘 憝 揎 摺`，以及 `忄 亻 扌 丌 丬 丶 丿` 这类
> 部首部件。用户输入「亻」或者打「摺」的时候同样是「这个字不存在」的体感，
> 所以按 GB2312 做兜底补字，拼音从 `pinyin-data` 取，**词频记 0**（排序时落在
> 每个码的最后，不影响常用字的位置）。
>
> **GB2312 是下限，不是上限。** GBK 里那两万多个字（大量异形字、繁体字、日韩
> 汉字）没有收：需要繁体时用「字形 = 繁体」由 `zhconv` 实时转，不需要在词库里
> 再存一份繁体字表；异体字属于「知道全拼也未必知道该用哪个」的范畴，收了反而
> 让候选变长。

真正的双拼码在**构建期**由 `build.rs` 算好：9.1 万条 × 三套码（小鹤/微软/全拼去空格），
各生成一张按码升序的下标表，连同偏移表和字节池写成一个扁平文件 `base.bin`，用
`include_bytes!` 嵌进 DLL。运行时只是「读两个 u32 数组 + 二分」，**零解析**。

这么做有两个直接好处：`Activate` 里原本 81ms 的编码开销归零；数据落在 DLL 的 `.rdata`
段，由 PE 加载器 mmap，**多个进程共享同一份物理页**，不再是每个进程各背 9.2MB 私有堆。

> 注意：不要换成 `luna_pinyin.dict.yaml`。它第三列是「多音字概率」不是词频
> （`丁 ding 99.93%`），按它排序会把生僻字排到最前面。

### 用户词库：txt 是源，bin 是产物

装完之后还能往里加自己的词，两种形态并存：

| 文件 | 角色 |
| --- | --- |
| `%APPDATA%\Feihe\user_dict.txt` | **源文件**，人可读、可手改、可导出（`词<TAB>全拼`） |
| `%APPDATA%\Feihe\user.bin` | 上面那份**编译出来的 FHDB 二进制**，运行时 mmap |

设置窗里三个按钮：`导入词库…` / `导出词库…` / `编译成二进制`。**导入后会自动编译**，
所以正常情况下你不用管 `user.bin`。

为什么值得编译：

| | 文本覆盖层（旧） | `user.bin` mmap（现在） |
| --- | --- | --- |
| 5 万条首次可用 | 每进程 124.6 ms | 每进程 **0.75 ms** |
| 内存 | 每进程 ~6 MB 私有堆 | 共享页缓存，全系统一份 |
| 查询 | 堆上二分 | 和 `base.bin` 完全同一条零解析路径 |
| 一次性成本 | 无 | 编译 139.9 ms（点按钮时付） |

判定规则是「产物不能比源旧」：如果 `user_dict.txt` 的 mtime 比 `user.bin` 新
（说明你手改了 txt 但没重新编译），运行时自动退回文本覆盖层，避免「改了不生效」。
点一次「编译成二进制」就回到快路径。

`user.bin` 和 `base.bin` 用的是**同一个编码器**（`src/dictenc.rs`，`build.rs` 与运行期
各以 `#[path]` 引入一次）。两边各写一份的话，改了布局忘改另一边，症状是「导入的词
静默不生效」，极难查。

> 跨进程生效时机：别的程序还映射着旧 `user.bin`。每个程序下次切换输入法
> （`Activate`）时会重新探测文件 mtime 并重映射，不用重启。

## 数据源与下载缓存

生成词库和辅助码表要拉外部数据，都缓存在仓库根目录，统一用 `_` 前缀，已被 `.gitignore`
忽略（`_*.yaml` / `_*.txt` / `_*.zip`）。**它们不是产品的一部分**——`build.rs` 只读
`src/dict.txt` 和 `src/assist.txt`，删掉缓存不影响编译。

| 文件 | 大小 | 谁读 | 删了会怎样 |
|---|---|---|---|
| `_icebase.yaml` | 15.8 MB | `tools/gen_dict.py` | 下次跑会自动重下（镜像可达） |
| `_ice8105.yaml` | 0.1 MB | `gen_dict.py`、`import_sogou.py` | 同上 |
| `_pinyin_data.txt` | 1.0 MB | `gen_dict.py`（GB2312 补字取音） | 自动重下（`pinyin-data` 的 `pinyin.txt`，多镜像） |
| `_ids.txt` | 2.1 MB | `tools/gen_assist.py` | 可重下（见下） |
| `_unihan.zip` | 8.0 MB | `tools/gen_assist.py` | **别删**：unicode.org 目前连不上，删了就没法重生成自然码辅助码 |
| `_luna.yaml` | 0.8 MB | **无人引用** | 死文件，可删 |
| `_kmandarin.txt` | 0.9 MB | **无人引用** | 死文件，可删 |

两个辅助码数据源**没有下载代码**，得手工放到仓库根目录：

```text
https://www.unicode.org/Public/UCD/latest/ucd/Unihan.zip          -> _unihan.zip
https://raw.githubusercontent.com/cjkvi/cjkvi-ids/master/ids.txt  -> _ids.txt
```

## 导入本机搜狗拼音的词库

飞鹤支持把你的搜狗个人词库搬过来——**打过什么词，飞鹤里就有什么词**。

```bash
python tools/import_sogou.py              # 自动发现并全量重建 src/user_dict.txt
python tools/import_sogou.py --list       # 只列出发现了哪些词库文件，不写
python tools/import_sogou.py --dry-run    # 只统计不写文件
python tools/import_sogou.py --max-len 8  # 放宽到 8 字短语（默认 6）
python tools/import_sogou.py --dir "D:\soft\sogoupinyin\用户数据\SogouPY.users\00000001"
```

**可反复执行**：每次都从搜狗原始文件全量重建，不累积、不残留。搜狗用久了想刷新，
直接再跑一次，然后 `build.ps1` + 替换 DLL 即可。

### 它读哪些文件

自动从注册表 `HKLM\SOFTWARE\WOW6432Node\SogouInput` 找安装目录，然后扫：

| 文件 | 内容 | 提取方式 |
|---|---|---|
| `用户数据/SogouPY.users/<id>/sgim_gd_usrbg.bin` | 背景词库（打字历史，主力） | 严格校验 |
| `用户数据/SogouPY.users/<id>/sgim_gd_usr.bin` | 用户词库 | 宽松提取 + 注音过滤 |
| `Scd/*.scel` | 细胞词库（要自己下载放进去） | 头部自适应解析 |
| `用户数据/SogouPY.users/<id>/*dict*.txt` | 手动导出的文本词库 | 按行取首列 |

### 格式怎么破的

搜狗没有公开文档，但实测词条是**明文 UTF-16LE**，只是外面套了私有二进制头：

- `sgim_gd_usrbg.bin` 的记录是 `[u16 字节长][UTF-16LE 文本][u16 字节长+6]`。
  最后那个字段等于「字节长 + 6」，正好当校验和用——能把字节错位产生的假词
  （`吐嚕`、`吀崀` 这类）全部滤掉，实测 5.3 万条命中、几乎无噪声。
- `sgim_gd_usr.bin` 尾部字段不是 `len+6`，退回「长度前缀 + 全 CJK」的宽松规则，
  噪声靠后面的「必须能注音」过滤。

### 注音策略（多音字为什么是准的）

1. 先在基础词库 `src/dict.txt` 里查——那里已经是「词 → 小鹤双拼码」，
   多音字天然正确（`重复`→`csfu`、`重要`→`zsyc`、`角色`→`jtse`）
2. 查不到才逐字注音：单字取 8105 字典里词频最高的读音，再用小鹤表编码

实测 5.2 万条里 3772 条命中基础词库、4.8 万条走逐字注音。逐字注音对多音字短语
（`重要`/`重复`）可能选错读音，这类词打不出候选是正常现象——按 `--max-len` 收窄
或手动改 `user_dict.txt` 都行。

### 两个词库怎么合并

分两个阶段，**不是运行时合并**：

1. **构建期**：开发机的 `src/user_dict.txt`（如果有）和 `src/dict.txt` 一起喂给
   `build.rs`，按词面去重、按码稳定排序，编成 `base.bin` 嵌进 DLL。
   发布构建（`FEIHE_NO_USER_DICT=1`）跳过前者——个人打字记录不能随包发出去。
2. **运行期**：`%APPDATA%\Feihe\user.bin`（由 `user_dict.txt` 编译而来）单独 mmap，
   查询时**先查用户词库、再查 `base.bin`**，用户词天然排在前面，不需要合并两张表。

两阶段都走 `dictbin` 的同一套零解析二分。查询成本是两次前缀区间定位，
和词库大小基本无关——`base.bin` 3.8 万条也好、8.7 万条也好，都是「读两个 u32 数组 + 二分」。

> 早期版本是运行时把两份文本解析进堆、合并、稳定排序，5 万条要 125 ms/进程、
> 每进程多背 6 MB 私有堆。换成「构建期预编码 + 运行期 mmap」之后降到 0.75 ms 且全系统共享一份页。

## 已知缺口

写在这里是为了让人一眼看到边界，而不是试了才发现。每一条都对照代码核实过。

**没做的功能**

| 项 | 说明 |
|---|---|
| 组合串下划线 | `GUID_DISPLAY_FEIHE` 已定义，`ITfDisplayAttributeProvider` 未实现 |
| 快捷输入扩展 | 已有日期 / 时间 / 星期 / 时间戳 / 农历 / 干支 / 节气，还没有自定义短语 |
| 模糊音 | 未排期。要做建议在 `shuangpin::encode_pinyin` 之后加等价类归并，而不是改词库 |
| 语音输入 | M5（v2）才规划，完全没动 |
| 删词 | 需要 SQLite（阶段二）。置顶已经做了（`pinned.txt`），删词还没有 |
| 不支持 TSF 的程序 | 独占全屏 DirectX 游戏等场景连组合串都没有，需要外挂键盘钩子工具，不做 |

**已知问题**

- 搜狗 `sgim_gd_usr.bin` 的宽松提取会带少量噪声词（字节错位产物），只在恰好打到对应码时出现
- 设置窗改双拼方案后，**其它已打开的进程**要重新激活才生效（`Activate` 里会重读配置，
  所以是「不即时」而不是「不生效」）
- 安装包未签名

## 验证清单

**M1（骨架）**

1. `build.ps1` 成功，生成 `feihe.dll`，且 `objdump -p` 的导入表里**没有** `libunwind.dll`
2. `register.cmd` 后 `HKCU\Control Panel\International\User Profile\zh-Hans-CN` 出现 `0804:{70CB823D...}{DA6B9170...}`
3. 键盘列表出现「飞鹤拼音」（列表由 shell 缓存，必要时注销重登）
4. 切到飞鹤后，`Ctrl+C` / `Alt+Tab` / `Win+Space` 等快捷键仍然正常（不被吞）
5. `unregister.cmd` 卸载干净，无残留

**M2（输入）**

6. 任意输入框敲 `nihc` → 组合串显示 `nihc`，光标下方弹出候选窗，第一个是「你好」
7. 空格上屏「你好」；敲 `vsgo` + 空格 → 「中国」；敲 `ui` → 候选里第一项「是」
8. 敲 `de` → 候选窗第一页是 的/地/得/德…，按 `3` 上屏「得」
9. 敲 `nih` 后连按 `=`（或 `↓`）能翻页，`-`（或 `↑`）回上一页，`←/→` 能改选中项
10. 敲 `nihc` 后 `Esc` → 组合串消失，文档无残留
11. 敲 `ni` 后退格两次 → 组合态结束，文档无残留
12. 敲到一半按 `Ctrl+C` → 组合串先提交，再执行复制

**用户词库（搜狗导入）**

13. `python tools/import_sogou.py` 输出「最终写入：约 5 万条」，`src/user_dict.txt` 约 1.1MB
14. 敲你平时常用、但基础词库里没有的短语（比如常打的短句），候选窗里能出
15. 用户词排在基础词前面：敲 `de`，如果搜狗词库里有以 `de` 开头的常用词，它应排在「的」之前
16. 反复执行 `import_sogou.py` 两次，`user_dict.txt` 内容一致（幂等）
17. 把 `src/user_dict.txt` 清空后重新构建，发布 DLL 回到 3.0 MB、`base.bin` 38024 条，
    说明没导入时不影响基础功能

**M3（32 位支持 / 长句分隔）**

18. `build.ps1` 默认产出两份 DLL，PE 头分别是 `Machine=0x8664` 和 `0x014c`，
    且两次 `[test] xx : PASS`（LoadLibrary + DllGetClassObject + CreateInstance）
19. `objdump -p` 里两份 DLL 的导入表都**没有** `libunwind.dll`
20. `register.cmd` 后 `HKLM\SOFTWARE\Classes\Wow6432Node\CLSID\{70CB823D...}\InprocServer32`
    指向 `%ProgramData%\Feihe\feihe32.dll`，且 `HKLM\SOFTWARE\Wow6432Node\Microsoft\CTF\TIP\{70CB823D...}`
    有完整的 Category / LanguageProfile
21. 在 32 位宿主里（例如 32 位 Office、`C:\Windows\SysWOW64\notepad.exe`）能用飞鹤打字
22. 打 `vhsjlisi` 时若词库无「张三李四」→ 进分词模式，`←/→` 能移段、数字能换词
23. 全拼模式下打 `nihaoshijie` → 切成 `[你好]世界` 两段（不再是整串挂字母）
24. `xi'an` 能强制切成 `xi`+`an`；`ni'hao` 的分隔符不会出现在上屏文本里
25. 微软双拼下 `;` 仍然出 `ing`，分隔用 `'`

**M3.5（交互打磨）**

26. 鼠标点候选窗里的词 → 直接上屏该项；按下后拖到窗外再松开 → 不上屏（防误触）
27. 点候选窗不会让输入框失焦，点完继续按键仍然进组合态（`WM_MOUSEACTIVATE → MA_NOACTIVATE`）
28. 敲单键 `n` → 候选窗只有 18 项（2 页）；敲 `ni` → 45 项（5 页）；三键及以上满 90
29. 设置窗勾掉「动态调频」→ 选词后候选顺序不再变化；点「重置」提示清空的条数
30. 打完一轮字后 `%TEMP%` 下**没有** `feihe-trace.log`（诊断日志已移除）
31. 打 `tsirll` 进分词显示 `[通传]两`，按退格 → `[通]传两`（候选变成「通」的字候选），
    再按退格 → 编码退回 `tsir`、候选出现「通传」（拆词那一步不丢编码）

**M3.6（农历 / 干支 / 节气 + 用户手册）**

32. 敲 `` `nl `` → 第一项形如「农历丙午年九月十四」，和手机日历对得上；
    闰月那一年（如 2023 闰二月）会显示「闰二月」
33. 敲 `` `gz `` → 四柱形如「丙午年 戊戌月 庚午日 壬午时」；立春前后各试一天，年柱会变
    （春节前后试则不变——年柱按立春换，不按春节换）
34. 敲 `` `jq `` → 给出下一个节气（如「2026年11月7日 立冬」）；当天正好是节气日则给当天
35. 只敲一个 `` ` `` → 8 个命令的当前值预览都能出来，`` `nl ``/`` `gz ``/`` `jq `` 在内
36. 安装目录下有 `使用帮助.md`，用记事本打开中文不乱码（UTF-8 BOM），卸载后一起消失

**M3.7（一级简码 / 手动置顶 / UILess）**

37. 敲单键 `d` → 第一个候选是「的」；`u`→是、`v`→这、`i`→出、`f`→非（26 键逐个试）
38. 敲单键 `d` → 候选**全是单字**（不再出现「都是 / 对了」这类两字词）
39. 敲 `de` → 第一个仍是「的」，但这时是词频在起作用，不是一简字
40. 打 `d`，按 `Ctrl+2` 选「大」→ 上屏「大」，且以后 `d` 的第一个变成「大」
41. 再打 `d`，对已经是第一的「大」按 `Ctrl+1` → 取消置顶，`d` 回到「的」
42. `%APPDATA%\Feihe\pinned.txt` 里能看到 `d<TAB>大`；手改成别的词、切一次输入法后生效
43. 设置窗显示「置顶 d→大 · Ctrl+数字 切换」，点「清空」后提示取消的条数
44. 敲 `d` 后立刻按 `Ctrl+C` → 组合串先提交，不误触发置顶
45. 设置窗勾上「UILess」→ 候选窗和状态浮窗都消失，组合串变成 `nihc  1.你好 2.拟好 …`
46. UILess 下空格出第一候选、数字键照样选、`-`/`=` 照样翻页
47. UILess 下按 `Ctrl+Shift+U` → 恢复候选窗和浮窗（这是唯一的键盘出口）
48. 关闭 UILess 后 `config.ini` 里 `uileless=0`，`config_ver=4`
49. 中文态敲 `/` → 组合串里出现 `/`，候选是八个命令的当前值预览
50. 敲 `/rq` → 第一个候选是今天的日期；`/r` 效果一样
51. 只敲 `/` 再按回车 → 上屏 `/`（开全角标点则上屏 `／`）
52. 中文态敲 `\` → 上屏顿号 `、`（不再有第二个键出顿号）
53. 敲 `/` 后按数字键 → 选的是候选（八个命令），数字不会被塞进组合串
54. 英文态敲 `/` → 正常出半角斜杠，不弹候选窗
