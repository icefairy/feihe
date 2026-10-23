# 飞鹤输入法 · 交接文档

> 记录**实测确认过**的事实和踩过的坑。凡是标「实测」的都是在本机跑出来的结论，
> 不要再凭直觉推翻。文档里没写「实测」的，都是待验证的推测。

## 当前状态

| 项 | 状态 |
|---|---|
| 注册链路 | ✅ 通（DLL 能加载进宿主进程，`Activate` 正常） |
| 按键接收 | ✅ 通（`AdviseKeyEventSink` 第三参数必须是 `TRUE`） |
| 组合态 + 候选窗 | ✅ 通（`nihc` → 候选窗 → 空格上屏「你好」） |
| 小鹤双拼编码 | ✅ 通（`ui`→是、`vsgo`→中国、`uh`→上） |
| 词库 | ✅ 8.18 万条预编码成 `base.bin` 嵌在 `.rdata`，零解析 + 跨进程共享页；整码优先排序 |
| 双拼方案 | ✅ 小鹤 + 微软两套表，词库运行时按方案编码（配置项 `scheme`） |
| 配置持久化 | ✅ `%APPDATA%\Feihe\config.ini` |
| 状态浮窗 | ✅ 中/英、全/半、符号、皮肤、设置五个按钮，可拖动、记住位置、可选跟随光标 |
| 设置窗口 | ✅ 双拼方案 / 辅助码方案 / 皮肤 / 全角 / 跟随 / 词库统计 + 导入导出 |
| 软键盘 | ✅ 9×5 特殊符号面板，点击走同步 edit session 插入 |
| 辅助码 | ✅ 自然码（部首声母 1 键）+ 小鹤音形（首末部件 2 键），Tab 切换 |
| 全角标点 | ✅ 不带 Shift 的那一排键做全角映射 |
| 全拼模式（临时） | ✅ 状态栏「全拼」按钮，独立全拼索引，不落盘 |
| 快捷输入 | ✅ `\` 引导：`\rq` 日期 / `\sj` 时间 / `\xq` 星期 / `\dt` 日期时间 / `\ts` 时间戳 |
| 鼠标点选候选 | ❌ 未做 |
| 组合串下划线 | ❌ 未做（`ITfDisplayAttributeProvider`） |
| Shift 中英切换 | ✅ 单击切（按住期间按过别的键就不算单击） |
| 回车原样上屏 | ✅ 空格选候选、回车把编码原样上屏 |
| 中英态生效 | ✅ TIP 自己在 `OnKeyDown` 里判定，不依赖宿主 |

## 自绘窗口架构（v2 新增）

三个自绘窗口 + 一个原生控件窗口，全部**在宿主进程的 UI 线程**上创建，
和 TIP 同线程，因此不需要跨线程 marshaling：

| 模块 | 窗口 | 类名 | 说明 |
|---|---|---|---|
| `candidate.rs` | 候选窗 | `FeiheCandidateWindow` | `WS_POPUP + NOACTIVATE + TOPMOST` |
| `statusbar.rs` | 状态浮窗 | `FeiheStatusBar` | 同上，另加 300ms 定时器做前台判定 |
| `softkbd.rs` | 符号软键盘 | `FeiheSoftKeyboard` | 同上，9×5 网格 |
| `settings.rs` | 设置窗 | `FeiheSettings` | 原生 BUTTON/STATIC 控件 |

配套：`theme.rs`（三套皮肤调色板，所有自绘窗口统一取色）、`font.rs`（雅黑 UI 13px，
懒加载 + `destroy()` 释放）、`insert.rs`（任意窗口往文档写文本 / 读写中英开关）。

### 几个必须知道的设计约束

1. **绝不抢焦点**。三个 popup 全部 `WS_EX_NOACTIVATE` + 处理 `WM_MOUSEACTIVATE`
   返回 `MA_NOACTIVATE`。一旦抢焦点，宿主文档失焦、组合串被打断。
2. **浮窗只在前台进程显示**。窗口是 topmost 且每个加载了本输入法的进程都有一个，
   不判断的话会多出一条条飘在屏幕上。用 300ms 定时器比对
   `GetWindowThreadProcessId(GetForegroundWindow(), None) == 创建窗口时的线程 id`。
3. **拖动和点击共存**。按下后位移 > 3px 才算拖动（移动窗口并写回 `bar_x/bar_y`），
   否则抬起时按按钮命中分发点击。没有标题栏。
4. **往文档写字走 `insert::insert_text`**。软键盘符号没有对应键位，`SendInput` 打不出来，
   必须 `RequestEditSession(TF_ES_SYNC)` + `SetText`。它需要 **client id**，
   由 `Activate` 里的 `insert::set_client_tid(tid)` 登记。
5. **`TF_GetThreadMgr` windows-rs 没导出**，用
   `CoCreateInstance(CLSID_TF_ThreadMgr = {529A9E6B-6587-4F23-AB9E-9C7D683E3C50})` 等价替代，
   拿到的就是**调用线程**的管理器。
6. **反激活时销毁所有窗口 + 字体**。窗口过程指针在 DLL 里，DLL 卸载后窗口还活着就是
   野指针崩溃。`Deactivate` 里顺序：`deactivate_sink()`（含候选窗）→ 浮窗 → 软键盘
   → 设置窗 → 字体。

## 辅助码（`assist.rs` + `src/assist.txt`）

数据由 `tools/gen_assist.py` 生成，3739 个常用字，两列码：

| 方案 | 规则 | 例 |
|---|---|---|
| 自然码辅助码 | 1 键 = 部首的拼音首字母 | 的→白→`b`、想→心→`x`、你→人→`r` |
| 小鹤音形 | 2 键 = 首部件 + 末部件，各按声母定键 | 打→扌(s)+丁(d)→`sd` |

自然码来源：Unihan `kRSUnicode`（部首编号）+ 214 个康熙部首的拼音首字母表。
小鹤来源：cjkvi-ids 的 IDS 分解（递归解析嵌套结构）+ `kMandarin` 定声母。

已知偏差：小鹤列是**按官方「声母定键」规则算出来的**，和官方码表在少数部件上有差异
（官方末形有时取笔画，如「形」官方 `kp`，这里 `ks`）。做候选筛选够用，
要完全对齐得换官方字根表。

词组按**首字**辅助码筛选，和主流输入法一致（`dict::lookup_filtered`）。

输入流程：正常敲双拼码 → `Tab` 进入辅助码态（组合串显示成 `code assist`）→ 继续敲
辅助码键缩小范围 → `Tab` 退出。辅助码关闭时 `Tab` 原样还给应用。

## 全拼模式

状态栏的「全拼」按钮给访客应急用，**故意不写进 config.ini**（`config::pinyin_mode()`
是一个进程内的 `AtomicBool`，进程结束即复原）。借电脑的人用完忘了切回来，下次开机不会
一脸问号。

实现上不重排词库：`Dict` 里额外挂一个 `OnceLock<Vec<(全拼去空格, raw 下标)>>`，
第一次切到全拼模式才建（约 60ms），之后一直复用。查询走 `dict::lookup_pinyin`。
`is_code_key` / `code_allowed` 在全拼模式下只认字母（微软双拼的 `;` 不再当编码键）。

## 词库架构（v3：静态二进制 + 运行时覆盖层）

### 源数据

`词<TAB>全拼`（音节空格分隔），按词频降序。**不存双拼码**——存码等于把方案焊死。
`src/dict.txt`（33500 系统词）+ `src/user_dict.txt`（52522 搜狗导入词）。

### 构建期：`build.rs` → `$OUT_DIR/base.bin`

把 8.17 万条预编码成三套码（小鹤 / 微软 / 全拼去空格），各生成一张**按码升序的下标表**，
连同「偏移表 + 字节池」一起写成一个扁平文件，用 `include_bytes!` 嵌进 DLL。

**为什么不用独立的 mmap 文件**：DLL 的 `.rdata` 段本来就由 PE 加载器 mmap，
**多个进程共享同一份物理页**。用 `include_bytes!` 就白拿了这个性质，还省掉
文件部署、路径解析、卸载清理三件事。

文件布局（全部小端）：

```text
0   "FHDB" | version u32 | count u32 | reserved u32
16  13 × (off u32, len u32)    各段偏移与长度
128 各段：5 张偏移表(u32[count+1]) + 3 张索引表(u32[]) + 5 个字节池
```

第 i 条的字节区间是 `pool[off[i]..off[i+1]]`；编不出码的条目是空区间，自然跳过。

实测（`llvm-objdump -h`）：`.rdata` 6.34MB（词库全在里面），`.data` 只有 3KB。

### 运行时

`src/dictbin.rs` 零解析：读头部拿到各段指针，查询 = 在索引表上二分，
比较时按需去池里取码。**没有解析、没有分配、没有排序。**

| | 旧（运行时编码） | 新（预编码） |
|---|---|---|
| `Activate` 开销 | 81ms，每个进程各付一次 | 0 |
| 常驻内存 | 9.21MB 私有堆 × N 进程 | 共享只读页，按需调入 |
| 切双拼方案 | 重建整个索引 | 换一张表 |
| 首次查询 | 81ms | 465µs（含页错误） |
| 后续查询 | 1.05µs | 3.2µs（多一层间接寻址） |

查询略慢一点但仍是微秒级，换来的是启动开销和每进程内存的双重收益。

### 用户词库：txt 是源，bin 是产物

`%APPDATA%\Feihe\` 下两个文件，**不要混为一谈**：

- `user_dict.txt` —— 人可读的源。导入/导出走它，用户手改也改它。
- `user.bin` —— 由它编译出来的 FHDB。运行时 `dictbin::map_file` 映射，
  和 `base.bin` 走同一条零解析二分路径。

实测（5 万条）：

| | 文本覆盖层 | `user.bin` mmap |
|---|---|---|
| 每进程首次可用 | 124.6 ms | **0.75 ms** |
| 内存 | ~6 MB 私有堆 × N 进程 | 共享页缓存，一份 |
| 一次性编译 | 无 | 139.9 ms（点按钮时付） |

四个实现要点：

1. **编码器只有一份**：`src/dictenc.rs`。`build.rs` 和运行期各自 `#[path]` 引入，
   格式天然一致。各写一份的话，改布局忘改一边 → 「导入的词静默不生效」，极难查。
2. **判据是「产物不能比源旧」**。`probe_user_bin()` 比较两者 mtime，txt 更新就退回
   文本覆盖层。否则用户手改 txt 后看不到效果，会以为坏了。
3. **映射永不解除**。`map_file` 返回 `&'static Bin`：视图和映射句柄故意不释放，
   进程退出时内核回收。这样避免引入自引用的所有权结构，代价是每次重编译残留一份
   共享页（重编译只在点按钮时发生）。
4. **改名覆盖已映射的文件是可行的**。重编译写 `user.bin.tmp` 再 `fs::rename` 覆盖，
   Windows 允许（旧映射继续指向旧文件）。`rename_over_mapped_file` 这个测试钉住了
   这个语义——不钉的话第二次编译会莫名其妙失败。

跨进程生效：别的进程还映射着旧文件，靠 `Activate` 里的 `dict::refresh_user_bin()`
重新探测 mtime（两次文件属性查询，可忽略）并重映射。

> 阶段二换 SQLite 之后，这个「txt 源 + bin 产物」的结构可以整体替掉；
> 现在它解决的是「装完还能加词，且不牺牲 bin+mmap」这一件事。

## 中英切换（`insert::CHINESE` + `textservice` 的 Shift 逻辑）

**根因**：`GUID_COMPARTMENT_KEYBOARD_OPENCLOSE` 写进去、读回来都对（状态栏标签会变），
但**宿主根本不拿它挡按键**。所以「切到英文还在输入中文」不是写失败，是 `OnKeyDown`
里压根没有判断这个状态。

现在的做法：进程内 `AtomicBool`（`insert::CHINESE`）当唯一真相来源，compartment 只是
同步给系统看的副本。

- `OnTestKeyDown` / `OnKeyDown` 开头都判 `insert::chinese()`，英文态一律 `return FALSE`
- `Activate` 里**只在中文态时**强制把 compartment 置 1。用户主动切到英文后不能拉回来，
  否则下一次激活会把他刚做的切换抹掉（这也解释了为什么不能无条件 `open_keyboard`）
- Shift 单击判定：`shift_used` 标记。`OnTestKeyDown`/`OnKeyDown` 里只要发现
  「非 Shift 键 + 物理 Shift 按住」就置位，Shift 抬起时没置位才切。
  **不能只在 `OnKeyDown(Shift)` 里标记**——经典宿主里 `OnTestKeyDown` 返回 FALSE 的键
  根本不会派发 `OnKeyDown`，Shift 自己就收不到

## 回车 vs 空格

- 空格 = 选当前候选
- 回车 = 把 `code` 原样上屏（`iuliqi` → `iuliqi`）
- 例外：`\` 开头的快捷输入没有「原样」可言，回车退回第一个候选（当前日期/时间）

## 整码优先（`dict::search`）

索引按码升序，所以「码恰好等于输入」的候选是从 `prefix_range().0` 开始的一段连续区间，
上界是 `exact_end()`。查询分两段走：先整码，再更长的词。

不加这个的话，打 `iuliqi` 会被词频更高的 `处理器吧` 抢走第一位。
（`处理器` 本身不在雾凇词表里，只能靠 `src/extra_dict.txt` 补。）

## 音节切分（`syllables` / `split_pinyin`）

所有「按音节处理」的功能都先经过 `FeiheTextService::syllables(code)`，它返回音节序列：

- **双拼**：定长 2 个字符一个音节（`syllables_ds`）。某个分组长度是奇数 → 返回空
  （音节还没打完，调用方「再等等」）。
- **全拼**：音节 1~6 字符不定长，用 DP + `shuangpin::is_syllable()` 校验
  （`split_pinyin`）。先算 `ok[i]`（`s[i..]` 能否完整切分），再贪心取**最长**音节
  ——`xian` 切成 `xian` 而不是 `xi`+`an`，`fangan` 切成 `fang`+`an`。
- 两种模式都先按 `'`（`SEP`）切组，用户手动指定的边界**一定**生效。

`is_syllable` 的实现是「拿小鹤表能不能编出双拼码」，不查词库。借小鹤表是刻意的：
它的零声母规则（必须 a/e/o 开头）正好是普通话的真实约束，比微软双拼「用 o 占位」更严格。
`ng`/`hm`/`hng` 这类叹词会被拒掉——它们几乎不出现在连续输入里，但会制造大量假边界。

## 边打边上屏（`maybe_split` + `EditOp::Split`）

**触发条件**（三个都要满足）：
1. 能切出**至少 2 个**音节（`syllables().len() >= 2`）
2. 整串一个候选都没有
3. 不是辅助码态、不是 `\` 快捷输入

满足时先试**分词模式**（`split_segments_units`，见下节）；切不出多段再退回
**拆分上屏**：`longest_prefix_units` 按音节边界递减找最长能出候选的前缀，
提交它的第一个候选，剩下的音节 `concat()` 作为新组合串。找不到就留给用户退格。

**为什么必须判「音节完整」**：小鹤 `vhsjl` 是 5 个字符 = 2.5 个音节，第三个音节的
韵母还没敲。此时拆会把用户正在输入的内容强行上屏，体验极差。

**实现要点**：`EditOp::Split` 在同一个 edit session 里做两件事——
先 `EndComposition` 把左边换成实际的字，再立刻 `StartComposition` 建新组合。
`EndComposition` 会**同步回调** `OnCompositionTerminated → clear_state()`，
把 `code` / `candidates` 清空。所以新 `code` 必须在 `EndComposition` **之后**赋值，
赋值前清空正好是无害的。`pending_op` 在 `run_edit_session` 开头已被 take，
不会被 sink 的重入清掉。

`split_segments_units` / `longest_prefix_units` 都把「有没有候选」抽象成
`impl Fn(&str) -> bool` 谓词，就是为了单元测试能注入假词典——否则测试要去改
进程级的 `SCHEME` / `pinyin_mode`，和 dict.rs 的测试并行跑会互相篡改
（实测踩过：`ms_scheme_lookup` 把 scheme 改成 Ms，导致小鹤码查不到）。
`dict::tests::SCHEME_LOCK` 是两边共用的那把锁。

## 分词模式（长句逐段选词）

`split_segments_units(units, has)` 的 DP 目标不是「切得最长」，而是**每一段都能出候选**。
纯贪心会把 `vhsjli` 切成 `vh`(张)+`sj`(三)+`li`(李)，因为 `vh` 单独就有候选。
先算 `ok[i]`（从第 i 个音节起能否切完），再在可行前提下贪心取最长段 → `vhsj`+`li`。

进入分词模式后：
- `composition_text()` 用 `[当前段]` 标光标位置，如 `[张三]李四`
- `refresh()` 在分词态下只查**当前段**的候选，并把选中项对齐到该段已选的词
- `←`/`→` 移段（`seg_idx`），`1-9` 换词（`confirm_segment`），空格确认并前移，
  回车整体上屏（`commit_segments`），退格退出（`exit_segment_mode`）
- 其他键：先整体上屏，键再交给应用

## 手动分隔（`;`）

`code` 里可以出现 `'`（`SEP`），它**只影响怎么切，不影响查什么**——
`refresh()` 用 `query_code()` 去掉它再去查词库，`commit_raw()` 上屏时也去掉。

键位按方案分叉，因为微软双拼的分号被 `ing` 占用了：

| 模式 | 分隔键 | 理由 |
|---|---|---|
| 全拼 / 小鹤双拼 | `;`（`VK_OEM_1`） | 编码里没有分号 |
| 微软双拼 | `'`（`VK_OEM_7`） | `;` 必须是 `ing`，否则 ing 打不出来 |

`can_insert_sep()` 拦三种无意义位置：编码为空、已有分隔符结尾、快捷输入态。
双拼额外要求断在**完整音节**之后（`tail.len() % 2 == 0`）——`vh'sj` 这种半截音节
会让整串切不出来，反而更难用。

非组合态按分隔键**不**开新组合串，落到全角标点分支（`;` → `；`）。

## 32 位支持（两份 DLL）

in-proc COM 的位数必须和宿主一致。同一个 CLSID 背后挂两个文件：

| 宿主 | DLL | 注册表 |
|---|---|---|
| 64 位 | `%ProgramData%\Feihe\feihe.dll` | `HKLM\SOFTWARE\Classes\CLSID\{...}` |
| 32 位 | `%ProgramData%\Feihe\feihe32.dll` | `HKLM\SOFTWARE\Classes\Wow6432Node\CLSID\{...}` |

三个必须知道的坑：

1. **用 `i686-pc-windows-gnullvm`，别用 `i686-pc-windows-gnu`**。官方 target 列表里
   **有** i686 gnullvm（早期误判成没有，走了弯路）。gnu 那条路会带 `rsbegin.o`，
   引用 `___register_frame_info` / `___deregister_frame_info`，llvm-mingw 用 libunwind + SEH
   没有这两个符号，链接直接失败，得写空实现垫上（本项目曾有个 `shim/eh_stub.c` 干这个，
   切回 gnullvm 后已删除）。装 std 时清华/阿里镜像不保留历史日期目录（404），
   用中科大：`RUSTUP_DIST_SERVER=https://mirrors.ustc.edu.cn/rust-static`。
2. **`HKLM\SOFTWARE\Microsoft\CTF\TIP` 是被 WOW64 重定向的**（CTF 下只有 `SystemShared`
   是共享键）。32 位进程读 `HKLM\SOFTWARE\Wow6432Node\Microsoft\CTF\TIP`。
   实测这台机器两个视图各有 26 个 TIP、内容完全一致，说明「镜像写两遍」是标准做法。
   只写 64 位那份 → 32 位宿主里飞鹤不出现在输入法列表。
3. **`build.rs` 的 windres 必须跟着 target 走**（`CARGO_CFG_TARGET_ARCH`）——
   用 x86_64 的 windres 编出来的 `.o` 塞进 i686 链接会报
   `machine type x64 conflicts with x86`。

实测：32 位 `i686-pc-windows-gnullvm` 产物的 `.text` 是 358 KB，比 `i686-pc-windows-gnu` 的
362 KB 略小；没有 `.pdata` / `.eh_frame` 段——i386 的 SEH 是运行时串异常记录链，
不需要表，这也反过来证明它走的不是 DWARF-2 展开。

`build.ps1` 收尾会做两件校验：PE 头机器类型（`0x8664` / `0x014c`），
以及用对应位数的 `shim/loadtest.c` 跑 `LoadLibrary` + `DllGetClassObject` + `CreateInstance`。
编译通过 ≠ 能被宿主加载——依赖 DLL 缺失、DllMain 崩溃、导出名写错，全都只在
`LoadLibrary` 那一刻才暴露。


## 安装包（NSIS）

`installer\feihe.nsi` + `installer\build_setup.ps1`。

```
powershell -ExecutionPolicy Bypass -File installer\build_setup.ps1
# → dist\Feihe-Setup-0.1.0.exe（约 1.16 MB，内嵌 5.4 MB 的 DLL）
```

设计取舍：

1. **注册逻辑不重写**。COM / CTF TIP / 用户键盘列表全部交给 `install_tip.ps1` /
   `uninstall_tip.ps1` —— 它们是唯一真相，`register.cmd` 用的也是同一份。安装包只负责
   「提权 + 拷文件 + 调脚本 + 卸载时反向调」。Pascal 里再写一套必然漂移。
2. **装到 `%ProgramFiles%\Feihe`**，不是老的 `%ProgramData%\Feihe`。安装时尽力清理老副本
   （`Delete /REBOOTOK` 处理被占用的）。
3. **发布模式构建**（`FEIHE_NO_USER_DICT=1`）。`src/user_dict.txt` 是 5 万条个人打字记录，
   普通构建会并进 `base.bin`（82179 条 → 33454 条，DLL 6.8 MB → 2.6 MB）。

五个坑（按踩到的顺序）：

- **`.nsi` 必须带 UTF-8 BOM**。NSIS 只在文件开头有 BOM 时才按 UTF-8 解析，否则按系统 ACP
  读。中文注释直接报 `Bad text encoding: feihe.nsi:1` + `Error in script "feihe.nsi" on
  line 1`，报错完全指不到真因。`build_setup.ps1` 现在编译前自动补 BOM。
- **DLL 必须 `File` 内嵌**。第一版用了 `CopyFiles`，那是**运行时**拷贝，源路径是构建机的
  `target\...`，用户机上不存在 —— 打出来只有 192 KB 空壳（`Install data: 73260 bytes`）。
  正确做法：`File` 解到 `$PLUGINSDIR\ime`，再 `CopyFiles` 到 `$INSTDIR`。
  验证手法：`Install data` 应等于 `DLL64 + DLL32 + 两个 ps1`（约 5,444,758 字节），
  多出的 ~55 KB 是 MUI2 自带的向导位图。
- **覆盖被占用的 DLL 只能「改名再拷」**。Windows 允许重命名已映射的文件，不允许覆盖。
  `CopyOrReplace` 先 `CopyFiles`，失败则 `Rename` 成 `.old` 再拷，仍失败则 `Delete /REBOOTOK`。
- **NSIS 是 32 位程序**，启动的 PowerShell 也是 32 位 → 写 `Wow6432Node\...` 会二次重定向。
  `install_tip.ps1` 开头检测 `[IntPtr]::Size -eq 4 -and $env:PROCESSOR_ARCHITEW6432`，
  切 `Sysnative\WindowsPowerShell\v1.0\powershell.exe` 重跑。
- NSIS 3.13 **无 `$PROGRAMDATA` 常量** → `ReadEnvStr $R0 "ProgramData"`。
  `makensis` 的 `/D` 值与 `D` 之间不能有空格；bash 里传 `-DOUTDIR=D:\...` 会被吃成
  `D://...`，要么走 PowerShell 要么用正斜杠。

安装包未签名，首次运行撞 SmartScreen。要消掉得买代码签名证书（EV 证书可即时过）。


## 关键事实（实测确认，别再猜）

| 事项 | 结论 |
|---|---|
| **`AdviseKeyEventSink` 第三个参数** | **必须传 `TRUE`**。官方文档：只有 TRUE 才把 sink 设为「前台 key sink」；「前台 key sink 接收所有键盘事件，非前台只接收 preserved key 和自己拥有文本上的按键」。传 FALSE → Activate 成功、sink 挂上，但 `OnTestKeyDown`/`OnKeyDown` 永远不被调用 |
| 中英文开关 | `GUID_COMPARTMENT_KEYBOARD_OPENCLOSE`（thread compartment）新注册 TIP 实测为 **0（关）**，需在 `Activate` 里置 1 |
| CoreText 宿主不调 `OnTestKeyDown` | WinUI3 应用（如 Win11 记事本）直接调 `OnKeyDown`，且会送来 Ctrl / 退格等任意键 → **`OnKeyDown` 必须自己完整过滤** |
| **小鹤声母** | **`zh→v`、`ch→i`、`sh→u`**（不是 z/c/s）。韵母 `ui` 也在 `v` 键上，与 zh 复用。`shi→ui`、`zhong→vs`、`chi→ii`、`shui→uv`、`zhui→vv` |
| **小鹤零声母** | **按韵母长度分三种**，不是一刀切：① 单字母 `a/o/e` 重复（`aa`/`oo`/`ee`）；② **双字母 `ai/ei/ao/ou/an/en/er` 保持全拼形式**（`ai`/`an`/`en`/`ou`）；③ 三字母 `ang/eng` 用韵母键（`ah`/`eg`）。所以「按钮」= **`annq`**。<br>⚠ 本项目早期把②也按「首字母 + 韵母键」算（`an→aj`），所有零声母词编码都错位，用户拿官方反查工具对出来的。依据：百度百科「小鹤双拼」/ 看云「小鹤入门 1.1 双拼」 |
| 小鹤反查工具 | `http://react.xhup.club/search`，官方反查。**编码有疑问先查它**，别凭直觉 |
| Win11 记事本 | `C:\Program Files\WindowsApps\Microsoft.WindowsNotepad_*\Notepad.exe`，WinUI3 应用，走 CoreText 通道，对 TIP 支持比传统 Win32 严格 |
| TIP 分类 | 只注册 `{34745C63...}`(KEYBOARD) 不够；WinUI3/UWP 应用还需 `{13A016DF-560B-46CD-947A-4C3AF1E0E35D}`(IMMERSIVESUPPORT) |
| 用户键盘列表位置 | `HKCU\Control Panel\International\User Profile\zh-Hans-CN`，值 `0804:{CLSID}{PROFILE}` = 顺序 DWORD |
| `InstallLayoutOrTip(psz, 0)` | **有效**，会同时写 User Profile + SortOrder |
| `InstallLayoutOrTipUserReg(psz, 0)` | **返回 FALSE**，不要用 |
| `UninstallLayoutOrTip` | input.dll 中**不存在**（104 个导出已全量核对） |
| 卸载正确姿势 | `InstallLayoutOrTip(psz, ILOT_UNINSTALL = 0x1)` |
| `ILOT_DEFPROFILE` | 0x2（设为默认） |
| `ILOT_NOAPPLYTOCURRENTSESSION` | 0x20（只存不生效） |
| 隐藏杀手 | `HKCU\...\CTF\TIP\{CLSID}\LanguageProfile\...\Enable = 0` 会让条目即使注册成功也不可见 |
| `Set-ItemProperty` | **没有** `-PropertyType` 参数（只有 `New-ItemProperty` 有）→ 用 `Set-RegValue` helper |
| 换 DLL 不用注销 | 重命名被占用的旧 DLL → 拷新的 → 新启动的进程自然加载新版；已加载的进程要重启 |
| `to_interface()` | 只在 `#[implement]` 生成的 `_Impl` 类型上，普通结构体上没有 → impl 块要拆开 |
| `TF_SELECTION.range` | 是 `ManuallyDrop<Option<ITfRange>>`，数组析构不释放 → 用完要手动 `take()` |
| 搜狗词库编码 | 明文 **UTF-16LE**，不是 UTF-8；文件头魔数 `SGPU`（usr）/ `SGBG`（usrbg） |
| 搜狗词条结构 | usrbg：`[u16 blen][text][u16 blen+6]`，尾部字段是天然校验和 |
| 搜狗词库位置 | `HKLM\SOFTWARE\WOW6432Node\SogouInput` 默认值 = 安装目录，用户数据在 `<安装>/用户数据/SogouPY.users/<id>/` |
| 词库合并 | `sort_by` 是稳定排序 → 先插用户词再插基础词，同码时用户词自然在前 |
| 词库性能 | 首次查询含全量解析约 11.5ms，后续 584ns |

## 调试手段

`src/log.rs` 是**临时诊断日志**，写 `%TEMP%\feihe-trace.log`，格式
`[毫秒][pid][ThreadId] 内容`。已在这些位置打点：

- `DllGetClassObject` / `CreateInstance`
- `Activate`（tid、focus、compartment 前后值、AdviseKeyEventSink 返回值）
- `OnTestKeyDown` / `OnKeyDown`（虚拟键码 + 判断结果）
- `DoEditSession`（Start/Update/Commit/Cancel + 失败原因）
- `candidate::show`（窗口坐标、尺寸、候选数）

**排查 TSF 问题的正确顺序**：先确认 sink 到底有没有被调用，再看组合态，最后才看候选窗。
不要一上来就怀疑 UI。

验收完成后把 `log.rs` 和所有 `crate::trace!` 调用删掉。

## 待办

- [ ] 候选窗鼠标点击选词（现在窗口吞掉点击但不透传）
- [ ] 组合串下划线（`GUID_DISPLAY_FEIHE` 已在 `clsid.rs` 定义，但没实现 `ITfDisplayAttributeProvider`）
- [ ] 快捷输入扩展：农历、`\` 后的自定义短语
- [ ] **阶段二**：用户词库换 SQLite（`winsqlite3.dll` + 手写 FFI），支持词频学习/删词/置顶
- [ ] 设置窗改双拼方案后，**其它已打开的进程**要重新激活才生效（方案缓存在各自的原子变量里）
- [ ] 删除诊断日志，回到干净构建
- [ ] `C:\ProgramData\Feihe\` 下 3 个 `feihe.dll.old-*` 被运行中的进程占用，重启后可删

已完成（别再当待办）：

- [x] Shift 单击切中英（`OnKeyUp` + 物理键态，见「中英切换」）
- [x] 全拼兜底（`config::pinyin_mode()` + 独立的全拼索引）
- [x] 回车原样上屏编码（`commit_raw`，快捷输入例外）
- [x] 整码优先（`dict::search` 两段查，`exact_end`）
- [x] 边打边上屏 / 分词模式（`maybe_split` + `EditOp::Split` + `segments`）
- [x] 全拼智能分隔（`split_pinyin` 的 DP + `is_syllable`）
- [x] `;` 手动分隔（微软双拼改用 `'`）
- [x] 32 位 DLL（`i686-pc-windows-gnullvm` + Wow6432Node 双份注册）
- [x] NSIS 安装包（`installer\feihe.nsi` + `build_setup.ps1`，发布模式构建）
- [x] 用户词库编译成 `user.bin` + mmap（`dictenc` 单一编码器 + 设置窗按钮 + 导入后自动编译）

## 已知问题

- 单键（如 `n`）会一次查出 90 个候选，偏多，可以按码长做衰减
- `sgim_gd_usr.bin` 的宽松提取会带少量噪声词（字节错位产物），只在恰好打到对应码时出现
