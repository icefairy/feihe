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
| 词库 | ✅ 9.1 万条预编码成 `base.bin` 嵌在 `.rdata`，零解析 + 跨进程共享页；整码优先排序 |
| 单字覆盖 | ✅ 8006 个（8105 表 7896 可编码全收 + GB2312 兜底补 110），GB2312 / 8105 缺 0 |
| 双拼方案 | ✅ 小鹤 + 微软两套表，词库运行时按方案编码（配置项 `scheme`） |
| 配置持久化 | ✅ `%APPDATA%\Feihe\config.ini` |
| 状态浮窗 | ✅ 中/英、全/半、符号、设置四个按钮，可拖动、记住位置、可选跟随光标 |
| 设置窗口 | ✅ 输入模式 / 双拼方案 / 辅助码 / 皮肤 / 字形 / 全角 / 跟随 / 动态调频 / 词库管理 |
| 动态词频 | ✅ `freq.rs`：选词计数 → 稳定重排候选，`freq.txt` 纯文本落盘，可关可重置 |
| 软键盘 | ✅ 9×5 特殊符号面板，点击走同步 edit session 插入 |
| 辅助码 | ✅ 自然码（部首声母 1 键）+ 小鹤音形（首末部件 2 键），Tab 切换 |
| 全角标点 | ✅ 不带 Shift 的那一排键做全角映射，新装默认开 |
| 简繁切换 | ✅ `zhconv`：LCMapStringEx + LCMAP_TRADITIONAL_CHINESE，逐字映射 |
| 全拼模式 | ✅ 设置窗「输入模式」单选，独立全拼索引，**落盘** |
| 快捷输入 | ✅ `` ` `` 引导：`` `rq `` 日期 / `` `sj `` 时间 / `` `xq `` 星期 / `` `dt `` 日期时间 / `` `ts `` 时间戳 / `` `nl `` 农历 / `` `gz `` 干支四柱 / `` `jq `` 下一个节气 |
| 鼠标点选候选 | ✅ `layout()` 统一几何，按下/抬起同一项才算点击，`MA_NOACTIVATE` 不抢焦点 |
| 候选数衰减 | ✅ 1 键 18 / 2 键 45 / 3 键以上 90（`candidate_limit`） |
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

设置窗「输入模式」里的双拼 / 全拼单选。`config::pinyin_mode()` 读的是一个 `AtomicBool`
镜像（每个按键上会被调用好几次，走 Mutex 不划算），`set_pinyin_mode()` 同时写盘。

**早期刻意不落盘**（状态栏上给访客临时用，怕借电脑的人切到全拼忘了切回来）。后来按钮
挪进设置窗，语义变成「设置项」——设置项改了重启就失效太费解，于是改成持久化。

实现上不重排词库：`Dict` 里额外挂一个 `OnceLock<Vec<(全拼去空格, raw 下标)>>`，
第一次切到全拼模式才建（约 60ms），之后一直复用。查询走 `dict::lookup_pinyin`。
`is_code_key` / `code_allowed` 在全拼模式下只认字母（微软双拼的 `;` 不再当编码键）。

## 标点：键位怎么分的

`fullwidth_punct(vk)` 只覆盖「不带 Shift 的那一排键」，Shift 组合一律交给应用自己算
（我们算的和键盘布局容易对不上）。

| 键 | 全角输出 | 备注 |
|---|---|---|
| `,` `.` | `，` `。` | |
| `\` | `、` | 搜狗习惯，**用户明确要求**；顿号只留这一个键 |
| `/` | —— | 快捷输入引导符，不是标点（`Shift+/` 出 `?`） |
| `;` | `；` | |
| `[` `]` | `【` `】` | |
| `'` | `’` | |
| `-` `=` | `－` `＝` | |
| `` ` `` | —— | 什么也不做，原样交给应用（曾经是引导符，已退役） |

### 翻页键

`page_delta(vk)` 把四组键收成一处：`-` / `↑` 上一页，`=` / `↓` 下一页。返回 `Option<isize>`，
`handle_page_key()` 调它并返回「键是否已吃掉」。

**只在组合态生效**（`handle_page_key` 在 `if !self.is_composing()` 之后才调用），
非组合态 `-` `=` 还是全角标点 `－` `＝`。组合态下哪怕只有一页也吃掉，行为才可预测——
否则「按了没反应」和「突然上屏一个＝」会随机出现。

放在 `match vk` **之前**而不是列四条 arm：match 模式里没法写 `VK_OEM_MINUS.0 as u8`
这种带转换的模式；而且分词模式（`on_segment_key`）要走同一套。

`is_code_key()` 里加了 `if Self::shift_down() { return false; }`，否则 `Shift+\` 会被
吞进编码，`|` 永远打不出来。字母不受影响——`vk.is_ascii_uppercase()` 那一条在前面就返回了。

**组合态下的标点要自己插。** 早期组合态走 `_ =>` 分支「先提交、把键还给应用」，结果
应用插的是半角 `,`，全角标点开关在组合态下形同虚设。现在 `_` 分支先看
`fullwidth_punct_for(vk)`（把「配置开关 + Shift 状态」两个门收在一处，两条路共用），
命中就 `commit_with(ch)` 把标点粘在候选后面并吞掉键。

### `pending_op` 只有一个槽位

`commit_current()` 和 `insert_direct()` 都往 `self.pending_op` 里 `replace`，然后
`request_edit_session()`。**连着调两个 = 前一个被冲掉**，表现为「字没上屏、只剩标点」。
所以「提交 + 粘标点」必须合成一个 `EditOp::Commit(text + punct)`，不能发两个 op。

## 简繁切换（`zhconv.rs`）

`LCMapStringEx(L"zh-CN", LCMAP_TRADITIONAL_CHINESE, ...)`，Windows 自带的 NLS 映射表。
零体积、零构建步骤，不用像常见做法那样塞一张 30 KB 的 `s2t.txt`。

- 纯 ASCII 走快路径直接返回 `Cow::Borrowed`（组合串、标点全走这条）
- API 失败 / 系统没中文 NLS 数据 → 原样返回简体，**绝不吞字**
- 幂等：繁体再转一次还是它自己，所以「候选窗转一遍 + 上屏再转一遍」不会出错

**调用点一律用 `apply()`，不要直接用 `to_traditional()`。** 后者是「无条件转」的
底层原语，`apply()` 才看 `config::traditional`。第一版就是四个调用点全直接调了原语、
谁也没判开关，结果选了「简体」照样打繁体（用户报的）。开关收在入口里，调用点不可能忘。

转换点只有三处（共四个调用），都不碰词库索引：
1. `candidate::label()` — 候选窗显示（每页 ≤9 个，每次重绘转一次，成本可忽略）
2. `composition_text()` 的分词分支 — 组合串预览
3. `run_edit_session` 的 `Commit` / `Insert` 分支 — 真正写进文档

第 3 条放在 `run_edit_session` 而不是各个 `commit_*` 里，是因为那里是所有上屏路径的
唯一出口（`commit_current` / `commit_raw` / `commit_segments` / `insert_direct` 都汇到它）。

## 农历 / 干支 / 节气（`quick.rs` + `tyme4rs`）

快捷输入的 `` `nl `` / `` `gz `` / `` `jq `` 三件事统一交给
[Tyme](https://6tail.cn/tyme.html) 的 Rust 版 `tyme4rs`（MIT）。**没有自己写一行历法**。

为什么不自研：这三样都是「看着简单、边界全错」的典型——

- 农历要处理闰月（2023 年闰二月）和大小月，得有一张 1900–2100 的数据表或者天文算法
- 干支四柱的**年柱按立春换**、**月柱按节气换**、**日柱按子时（23:00）换日**，
  三条规则各错一次就是三种不同的 bug，而且只有生日当天才看得出来
- 节气是太阳黄经每 15° 一个，要算真太阳位置，不是固定日期

自己写等于重造万年历。Tyme 的节气算法引自寿星天文历，农历/干支/节气共用同一套
儒略日，三者天然一致。

**代价：+127 KB（64 位 DLL 2880 → 3007 KB）**。tyme4rs 依赖 `regex`（作者用它解析
中文名称），而 regex 本身就占大头。安装包 1.24 → 1.36 MB。为了历法正确性，这个体积
值得付；如果哪天要抠，唯一办法是 fork 掉 regex 那三处 `from_name` 解析。

### 三个实现细节

1. **`get_term_day()` 给的是「今天所在的节气 + 已过天数」**，不是下一个。
   `day_index == 0` 说明今天就是节气日，那就是最近的一个；否则 `next(1)` 再取日期。
2. **`` `nl `` 和 `` `gz `` 的年不一样，是对的**：前者用 `LunarDay` 的农历年干支
   （春节换年），后者用 `EightChar` 的年柱（立春换年）。别为了「统一」把它们改成一样。
3. **按分钟缓存**。实测 `lunar` 176µs、`ganzhi`（EightChar）**754µs**、`term` 11µs，
   而 `candidates()` 每敲一个键都会重算，只打一个 `` ` `` 时更是要把 8 个命令全列出来。
   所以 `CACHE` 按「年月日时分」把三份结果一起缓存，命中后 17µs。
   分钟变了才重算——农历只到日、时柱两小时才换，秒级差异不影响输出。

> 注意 `LunarDay::get_year_sixty_cycle` 已废弃，走 `get_sixty_cycle_day().get_year()`。
> 另外 Tyme 用「十二月」不用「腊月」，跟着库的措辞走，别自己改一套。

## 动态词频（`freq.rs`）

静态词频是 rime-ice 通用语料排出来的，代表「大众习惯」，解决不了「我的高频词」。
学习结果**不能写回 `base.bin`**——那是 `include_bytes!` 嵌在 `.rdata` 的只读数据，
多进程共享物理页，改不动也没意义。所以单独一份 `%APPDATA%\Feihe\freq.txt`：

```text
# 飞鹤输入法动态词频
# 格式：词<TAB>次数（删掉本文件即重置）
诊断	37
函数	12
```

### 三个关键决定

**1. 只学「从候选里选中的词」。** 触发点是 `commit_with`（选候选 / 空格 / 标点）
和 `commit_segments`（分词逐段选）。回车原样上屏（`commit_raw`）、快捷输入、
软键盘符号都不学。`bump` 里还有一道 `word.is_ascii()` 兜底：一个候选都没有时
`current_word()` 会退化成原始双拼码（`nihao`），那不是词。

> 注意全角标点**不是** ASCII，所以那道判断挡不住 `，`。标点之所以不会被学，
> 靠的是调用约定：`commit_with` 传的是选中的候选词，标点只是拼在后面的 suffix。
> 这点很容易想当然，测试 `ascii_is_not_learned` 里写明了。

**2. 排序必须稳定。** `sort_by_key` 是稳定排序，键是次数降序：

```rust
words.sort_by_key(|w| std::cmp::Reverse(table.get(w.as_ref()).copied().unwrap_or(0)));
```

没学过的词相对顺序**一点都不能变**。要是图快用 `sort_unstable`，所有码的候选顺序
都会抖，用户第一反应就是「输入法乱套了」。

**3. 排序发生在截断之后。** `dict::search` 先按 `limit` 收满候选再重排，
所以学习记录只能把「本来就在候选里」的词往前挪，不能把第 90 名以后的捞进来。
全量扫描能让极冷僻词被选一次就上首页，但代价是每次按键多扫几万条，不值。

### 性能

`rank` 每次按键都跑，所以**绝不克隆学习表**——直接持 `RwLock` 读锁排序，
写锁只在 `bump` 时短暂争用。5 万条上限，`bump` 一次是 O(1) 哈希插入。

### 落盘策略

`bump` 只改内存并记脏，**最多 3 秒写一次**（`FLUSH_INTERVAL`）。
每次上屏都写文件既没必要也伤盘；只在退出时写又不可靠——TSF 宿主进程什么时候退出、
`DllCanUnloadNow` 会不会被调，都不由我们说了算。丢最多 3 秒的学习量，可以接受。

写盘和 `config::save` 一样走「临时文件 + 改名」，多进程同时写不会读到半截文件。
`cfg!(test)` 时直接 return，免得 `cargo test` 把用户真实的 `freq.txt` 冲掉。

### 多进程：这是这个功能最容易做错的地方

每个宿主进程（记事本、Word、浏览器…）都加载一份本 DLL，各有**独立的内存表**。
天真实现会有两个症状，而且都很难复现：

1. **互相覆盖**：记事本写完盘，Word 再写一次，就把记事本学的词冲掉了
2. **时灵时不灵**：在 Word 里把「诊断」选到第一位，切到浏览器里它又沉回去

对应两个措施：

- `flush()` 写盘前先 `parse` 磁盘文件、`merge_into` 内存表。合并规则是
  **同词取 max 不相加**——两个进程各数到 5 和 3，描述的是同一段历史，
  真实次数更接近 5；相加会随进程数膨胀。
- `freq::refresh()` 挂在 `Activate` 上（和 `dict::refresh_user_bin` 并排），
  按文件 mtime 判断有没有别的进程写过，变了才读盘合并。
  `Activate` 是高频动作，不能每次都解析几千行文本。

> 这不是「完美一致」，是「足够好」：真正的强一致要文件锁或 SQLite 事务。
> 但学习表丢几条计数无伤大雅，而上文件锁会让每次上屏都可能阻塞在 I/O 上，
> 得不偿失。等阶段二换 SQLite 时再一起解决。

### 配置迁移的坑（`config_ver` 3）

加 `learn` 字段时把 `CONFIG_VER` 从 2 提到 3，顺手差点踩雷：原来的写法是

```rust
if ver < CONFIG_VER { cfg.fullwidth = true; }   // v1→v2 的迁移
```

只有 v1→v2 时完全正确；一旦 `CONFIG_VER` 变成 3，**v2 用户明确关掉的全角标点
会被再迁移一次、强行改回 true**。所以改成按阶段精确判断 `if ver < 2`。
回归测试 `v2_migration_does_not_touch_fullwidth` 钉死这条。

> 教训：迁移条件永远写「小于哪个版本」，不要写「小于最新版本」。后者在加下一个
> 版本时会静默重放所有历史迁移，用户设置被无声改掉，比崩溃还难查。


## 候选窗鼠标点选（`candidate.rs`）

候选窗是自绘的 `WS_POPUP`，本来就把点击吞掉了——「点了没反应」比「点了没这个功能」更难受。
实现里有四个地方值得记一笔：

**1. `#[implement]` 没有反向路径。** 它只生成「结构体 → 接口」的转换，
拿到 `FeiheTextService_Impl` 的裸地址后没法再还原成 `&FeiheTextService`。
而点击回调需要调 `_Impl` 上的私有方法（`pick` / `confirm_segment`），
所以只能存裸地址 + 一个 `Arc<()>` 的 `Weak` 当存活令牌：对象析构后 `upgrade()` 失败，
回调直接返回，不会打到野指针上。

```rust
let addr = self as *const FeiheTextService_Impl as usize;
let alive = Arc::downgrade(&self.alive);
candidate::set_click_handler(addr, move || {
    let alive = alive.clone();
    Arc::new(move |index| {
        let Some(_token) = alive.upgrade() else { return };
        let this = unsafe { &*(addr as *const FeiheTextService_Impl) };
        this.on_candidate_click(index);
    })
});
```

**2. 命中区必须和绘制用同一份几何。** `layout()` 返回总宽 + 每项的 `(left, right)`，
绘制和 `hit()` 都只认它。否则改个 `PAD` 或字体就变成「看着点到了、实际没中」，
而且这种错只在特定词长下出现，很难对上。

**3. 按下 + 抬起落在同一项才算点击。** `WM_LBUTTONDOWN` 记 `pressed` 并 `SetCapture`，
`WM_LBUTTONUP` 比对后分发，中途 `WM_CAPTURECHANGED`（拖出去、被别的窗口抢）就清空。
只认 `WM_LBUTTONUP` 会让「按下发现点错了、拖出去松开」变成误上屏。

**4. 回调不能持锁调用。** 回调会走 edit session，可能又触发 `show()` → `set_click_handler()`，
持着 `ON_CLICK` 的锁调用就是自锁。所以先克隆 `Arc`、释放锁，再调。

> `SetCapture` / `ReleaseCapture` 归 `Win32::UI::Input::KeyboardAndMouse`，
> 不在 `WindowsAndMessaging`，`windows` crate 的模块划分和直觉不一致。
> 另外窗口要响应 `WM_MOUSEACTIVATE` 返回 `MA_NOACTIVATE`，否则点候选窗会把
> 输入框的焦点抢走，点完继续打字就打到别处去了。

## 候选数按码长衰减（`textservice.rs`）

```rust
fn candidate_limit(code: &str) -> usize {
    let n = code.chars().filter(|c| *c != SEP && !c.is_whitespace()).count();
    match n { 0 | 1 => 18, 2 => 45, _ => MAX_CANDIDATES }   // 90
}
```

单键（`n`）能命中几千个字，取前 90 个里绝大多数是用户永远不会选的。
18 = 2 页，正好覆盖「我要的那个字在第二页」这种常见情况；三键以上才可能真的需要 90。

改的是一个函数，但所有查询入口都得换：`refresh` 的两条路径、`confirm_segment`、
分词 `lookup_any`。漏一处就会出现「分词时又冒出 90 个」。

## 词库架构（v3：静态二进制 + 运行时覆盖层）

### 源数据

`词<TAB>全拼`（音节空格分隔），按词频降序。**不存双拼码**——存码等于把方案焊死。
`src/dict.txt`（38573 系统词）+ `src/user_dict.txt`（52522 搜狗导入词）。

**单字必须全收，不能按词频截断。** 早期 `gen_dict.py` 里 `MAX_CHARS = 3500`，
把 8105 字表按词频砍到前 3500 个。某个生僻姓在表里排 5402 名（词频 796，
第 3500 名的下限是 11418）——直接没了，用户翻页也翻不到。
现在 `MAX_CHARS = None`（8006 个可编码单字全收，8573 条含 567 个多音字读音）。
回归测试：`rare_name_chars_are_reachable`（仝佟覃翟邬阚亓郗禚仉）。

**GB2312 兜底补字。** 《通用规范汉字表》不含全部 GB2312 二级字，实测缺 110 个
（`嬲 欷 榘 憝 揎 摺` + `忄 亻 扌 丌 丬 丶 丿` 这类部首部件）。`gen_dict.py` 里的
`gb2312_hanzi()` 枚举全部 6763 个 GB2312 汉字，`load_pinyin_data()` 读
`pinyin-data` 的 `pinyin.txt`（`U+4E00: yī,yí,yì  # 一`，44440 行）取音，
只补「整个字都没收录」的（避免多音字被塞进别的码），词频记 0。

覆盖率（生成后自检）：
| 字表 | 字数 | 覆盖 | 缺 |
|---|---|---|---|
| 通用规范汉字表 | 8105 | 7896 可编码全部 | 0 |
| GB2312 | 6763 | 6763 | 0 |
| 去重合计 | — | 8006 | — |

GB2312 是**下限不是上限**：GBK 那两万多字（异形字 / 繁体字 / 日韩汉字）不收，
繁体靠 `zhconv` 实时转，异体字收了只会让候选变长。

补字后 `base.bin`（发布构建，不含个人词库）**2321 KB / 38024 条**，补字前 2316 KB ——
5 KB 换 110 个字，这个交换没有任何犹豫的余地。（`dict.txt` 是 38573 行，`build.rs` 里
按「词 + 全拼」去重后剩 38024 条。）

### 构建期：`build.rs` → `$OUT_DIR/base.bin`

把 9.1 万条预编码成三套码（小鹤 / 微软 / 全拼去空格），各生成一张**按码升序的下标表**，
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

## 前缀排序（`dict::top_by_rank`）

**这套东西把原来的「整码优先」换掉了。** 原实现用 `exact_end()` 二分出「码恰好等于
前缀」的一段，先出这一段，再出更长的词。它有个没想到的副作用：

索引是按**码**升序排的，跨音节时按码排 = 按字母排。打单键 `d` 时没有任何「码恰好
等于 d」的条目，于是整段都算前缀匹配，而 `da` 在字母序上排在 `de` 前面——
18 个候选位全被「大/打/答/达」占满，「的」（`de`）一个都进不来。单键打字等于废掉。

现在的排序键是 `(码长, 原始行号)`：

- **码长短的优先**——单键 `d` 下两码条目正好是一个整音节（单字），四码是两个音节（词）。
  打一个键就只出单字，这符合直觉
- **行号次之**——`dict.txt` 是按词频降序写的，行号就是词频排名。同码长内按它排

顺带就覆盖了原来的「整码优先」：`nihc` 下整码的「你好」码长 4，比「你好吗」的 6 短，
自然排前面。`exact_end()` 因此删掉了。

两个坑：

1. **不能只按行号排。** `base.bin` 是「个人词库 → rime-ice → 补充表」拼起来的，
   个人词库那段在最前。只按行号的话 `d` 的前 18 名会变成「都是 / 对了 / 都有」，
   单字一个不剩。这就是为什么要先比码长。
2. **单键是最贵的一档**：区间跨 20 多个音节、几千条，`top_by_rank` 要整段扫。
   实测（86749 条词库，release）单键 58µs、三键 6µs。`rank_key()` 一次取出
   `(码长, 行号)` 就是为了少读一遍索引表——分开取会多一倍随机访问。

## 一级简码（`brief.rs`）

26 个键各配一个一简字，单键查询时提到第一位。表是小鹤音形官方那 26 个字
（「去我二人他 / 一是出哦平 / 啊三的非个 / 和就可了 / 在小才这 / 不你没」）。

容易记错的地方：`f`=非（不是「发」）、`v`=这（不是「中」）、`x`=小（不是「下」）。
`u`/`i`/`v` 分别是是/出/这，因为小鹤双拼里它们兼 `sh`/`ch`/`zh`。

实现上只做「**提上来**」，不做「**塞进去**」——`dict::search` 里 `insert_missing = false`。
理由：微软双拼键位和小鹤不同（`u` 不再是 `sh`），开辅助码筛选时一简字也可能被筛掉，
这两种情况硬塞一个不相干的字比不生效更糟。

## 手动置顶（`pin.rs`）

`Ctrl+数字` = 置顶该候选并上屏，对已置顶的那个再按一次 = 取消。
存在 `%APPDATA%\Feihe\pinned.txt`（`编码<TAB>词`）。

优先级：**置顶 > 一简字 > 自造词 > 动态词频 > 静态词频**。所以 `dict::search` 里的顺序是
`freq::rank()` → 提自造词 → 提一简字 → 提置顶，谁后提谁赢。

置顶 `insert_missing = true`：词排到第 90 名以外也要钉上来。这是它和动态词频最本质的
区别——后者排序发生在 `limit` 截断**之后**，只能重排已经在列表里的词。

几个实现细节：

- `Arc<str>` 存词面，`word_for()` 克隆只加一次引用计数（每次按键都走这条路径）
- 写盘前先 `parse` 磁盘内容 `or_insert` 合并，理由同 `freq::flush`（多进程互相覆盖）
- `refresh()` 挂在 `Activate`，按 mtime 判断；文件被删时把内存表也清掉
- 分词模式下钉的是**当前段**的码，不是整串——否则下次打同一个词对不上

## 自造词（`phrase.rs`）

用户的原话：「我用 `limkyr` 逐字组成的『李明远』，再次打 `limkyr` 并不能出现我刚组出来的词」。

这是**自动造词**缺失，不是动态词频的 bug。`freq` 只能重排**已经在候选里**的词；
而「李明远」压根不在词库里——它只在用户那一次分词上屏里存在过，上屏完就没了。
所以下次敲同样的码，走的还是老路：自动分词 → `[李明]远`。

**触发点**：`commit_segments` 里调 `remember_phrase` → `compose_phrase`（纯函数）。
四道门（缺一条就开始往表里灌废话）：

1. 至少两段——单字不叫组词，单字的排序 `freq` 已经管了
2. 每段都真有词——空段是 `merge_segments` 兜底塞的段码，不是用户选的
3. 段码拼起来 == 用户敲的那串——中途改过编码，码和词已经对不上
4. 词库里本来就有 → 跳过。自动切分沿用默认候选（用户没动）不该记

**存全拼，不存码。** 这是和 `pin` 最大的区别：置顶存 `编码<TAB>词`，换方案那条就废了
（用户自己钉的，认了）；自造词是**自动**记的，用户不知道有这回事，换方案后突然不灵
只会当成 bug。所以存 `词<TAB>全拼`，查询时按当前方案现场编码，小鹤/微软/全拼三套索引
都对得上。

全拼靠 `dict::pinyin_of(码, 词)` 逐段反查（词库里存的就是全拼）。**必须整码匹配**：
`li` 这个前缀下还挂着 `limk`（李明）这类更长的条目，只按前扫会把长词的全拼套给
单字「李」，记下来的词下次打不出来——这种静默失效最难查。任何一段查不到就整体放弃。

**排序位置**：`dict::search` 里在 `freq::rank` **之后**、一简字**之前**。一简字是肌肉
记忆、置顶是用户显式钉的，两者都该赢过自动学来的东西。`insert_missing = true`
（自造词本来就不在候选里，必须塞进去）。带辅助码筛选时不参与——辅助码是用户主动
「再筛一道」，自造词没有可靠的首字辅助码。

`phrases.txt` 里越靠前越新；同一个词再学一次只是挪到最前，不产生重复条目。

## 不弹窗模式

配置项 `no_popup`（`config_ver=5`；v4 及以前叫 `uileless`，`parse` 仍认旧键）。
开关做成原子量 `config::no_popup()`，因为候选窗每次按键都要问一遍。

**为什么改名。** 这功能原来叫 UILess，但它和 TSF 真正的 UILess（宿主自绘候选）是两件
完全不同的事，只是都「不弹窗」。真 UILess 落地后必须拆开：现在 `UILess` 专指宿主驱动
的那条（`src/uielement.rs`），开关叫「不弹窗模式」。`Ctrl+Shift+U` 这个键位留着没动
（老用户的肌肉记忆），注释里写清了它跟真 UILess 无关。

拦截点在 `candidate::show()` 和 `statusbar::show()`——它们是所有显示路径的**唯一出口**。
拦在上层（`textservice::show_candidates`）的话，将来加个调用点就漏了。

**候选必须塞进组合串。** 双拼同码字多（`de` 有几十个），只给「空格出第一候选」等于
盲打。组合串是宿主画在文档里的、不是我们弹的窗口，全屏程序里通常还能显示，
所以那是这条路下唯一能给用户看的通道：`nihc  1.你好 2.拟好 3.你号`。
宿主接管 UILess 时反过来要**关掉**它（`no_popup_hint` 里先判 `ui_less`），否则屏幕上
会出现两份候选。

**必须有键盘出口。** 浮窗上的「设置」按钮是进设置窗的唯一入口，藏了浮窗就把用户锁在
外面。所以 `Ctrl+Shift+U` 切回，而且必须拦在「修饰键一律交还应用」那条之前。

## TSF UILess（宿主自绘候选）

`src/uielement.rs`。触发条件只有一个：宿主用
`ITfThreadMgrEx::ActivateEx(TF_AE_UIELEMENTENABLEDONLY)` 激活线程管理器，TSF 把同一个
`dwflags` 透传给我们的 `ITfTextInputProcessorEx::ActivateEx`。**没有任何用户开关。**

链路：`show_candidates` → `Session::update` → 首次 `BeginUIElement(el, &pbShow, &id)`。

- `pbShow == FALSE` = 宿主接管，我们一个窗口都不开，靠 `UpdateUIElement(id)` 推数据；
- `pbShow == TRUE` = 宿主不接，立刻 `EndUIElement` 收回，本轮激活把 `ui_less` 清掉降级
  成自绘窗（否则每次按键都要白绕一圈 COM）。

实现上三个必须记住的点：

1. **`UpdateUIElement` 是同步回调**，宿主会立刻回头问 `GetString`。所以数据必须先在
   锁内改完、放锁、再调 COM。`textservice` 的 `uielement` 字段存 `Rc<Session>` 就是
   为了「克隆句柄 → 放借用 → 再调用」，否则 `RefCell` 上重入直接 panic（panic 跨 FFI
   边界就是 abort，输入法直接没了）。
2. **`GetString` 返回候选词本身**，不带 `1.` 序号——序号是宿主排的。简繁转换在这里做
   （和 `candidate::label` 一个规矩：存词库原文，显示层才转）。
3. **每次组合重建元素对象**（`BeginUIElement` 时新建）。复用同一个在部分宿主上会把
   上一轮的候选带过去。

`GetUpdatedFlags` 一律返回全量六个标志。老实算差集要维护一份快照，收益为零——宿主拿到
全量只会多读几次 `GetString`，而候选本来就是整页换掉的。

`ITfCandidateListUIElement` 继承 `ITfUIElement`，两个 `_Impl` 要分开写（和
`ITfTextInputProcessor(Ex)` 的处理方式一样）；`#[implement]` 里两个接口都列上，保证宿主
QI `ITfUIElement` 也能成功。

`TF_AE_UIELEMENTENABLEDONLY`（`0x1`）在 windows-rs 里**没有导出**（`TF_AE_NONE` 这个名字
被 `TfActiveSelEnd` 占了），只能自己定义，所以加了个单测钉住它的值——写错了症状是
「游戏里没候选」，极难看出是常量问题。

`IUISimpleLayout` 没实现：windows-rs 0.62 的 TextServices 里根本没有这个接口，
而且宿主不用它也能正常翻页（页边界走 `GetPageIndex`）。

边界：程序如果既不走 UILess、也不显示组合串（极少数完全自绘输入框的引擎），飞鹤在里面
一个字也打不出来。那需要外挂键盘钩子 + `SendInput` + 注入进程画候选，会直接撞反作弊，
不在本项目范围内，文档里写清楚了。

## 删除类操作不能走「合并磁盘」

`freq::flush` 写盘前会先把磁盘内容并进内存表（`merge_into`：同词取大、新词插入），
这是为了多进程不互相覆盖。但**合并是 `or_insert` 语义，删除类操作会被它废掉**：

- `freq::clear()` 清空内存 → `flush()` 把磁盘 100 条合并回来 → 写回 100 条。
  用户点「重置」什么也没发生，而且完全静默。
- `pin::remove()` 同理：删掉一条，`save()` 又把它 `or_insert` 回来。

所以：

- `freq` 拆出 `flush_inner(merge_disk: bool)`，只有 `clear()` 传 `false`。
- `pin` 干脆不用合并那套：每次写都「读盘 → 改 → 写盘」（`mutate`）。置顶表只有
  几十行，整读整写毫无压力，换来的是增删改全部正确。
- `refresh()` 也从「合并」改成「整份替换」——否则别的进程刚清空，本进程的
  `merge_into` 会把旧内容留在内存里，清空永远传不过来。

## 回车 vs 空格

- 空格 = 选当前候选
- 回车 = 把 `code` 原样上屏（`iuliqi` → `iuliqi`）
- 例外：`` ` `` 开头的快捷输入没有「原样」可言，回车退回第一个候选（当前日期/时间）

## 整码优先（已并入「前缀排序」）

原来是 `dict::search` 里先用 `exact_end()` 分出「码恰好等于输入」的一段再出更长的词，
不加的话打 `iuliqi` 会被词频更高的 `处理器吧` 抢走第一位。
（`处理器` 本身不在雾凇词表里，只能靠 `src/extra_dict.txt` 补。）

这套规则现在被 `top_by_rank` 的 `(码长, 行号)` 排序统一覆盖了，`exact_end()` 已删除。
详见上面的「前缀排序」一节。

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

## 不自动上屏（`maybe_split` / `try_segment`）

**只有显式动作才写文档**：空格、数字、回车、标点。敲字母永远只改组合串。

早期版本有「边打边上屏」：整串没候选时，把左边能匹配的部分直接上屏、
剩下的编码开新一轮（`EditOp::Split`）。设计初衷是「别挂着一串没人可选的字母」，
实际是灾难——用户打 `viuiq` 想输入「指示器」，屏幕上凭空出现「只是」，
后面的 `qi` 又变成一个新的单字组合。用户原话：

> 我没有敲空格就不应该上屏任何字或者词，应该让我可以挨个选

`EditOp::Split` 已整个删除。现在 `maybe_split` 只做一件事：切成多段、进分词模式。
切不出来就什么都不做，等用户继续敲或退格。

### 触发条件

音节完整（双拼偶数长度 / 全拼能切出合法音节）、至少 2 个音节、整串候选为空、
不是辅助码态、不是 `` ` `` 快捷输入。

### `plan_segments`（纯函数）

分段决策抽成 `FeiheTextService::plan_segments(code, has)`，不碰 `self`。
原因是 `#[implement]` 生成的 `FeiheTextService_Impl` **在测试里构造不出来**
（windows-rs 只生成 `From<Foo> for IInterface`，没有 `From<Foo> for Foo_Impl`），
所以状态机没法拿实例测，决策必须做成纯函数才能覆盖。

策略：先按「每段都能出候选」做 DP；DP 失败（某个音节词库里完全没有）就退到逐音节。
逐音节可能有个别段空着，但至少用户看得见、能退格——总比替他上屏强。

### 分词模式下敲编码键

`on_segment_key` 遇到编码键**延长编码并重新分段**，不 commit：

```rust
self.code.borrow_mut().push(ch);
self.segments.borrow_mut().clear();
self.refresh();
if !self.try_segment(&prev) { self.update_composition(); }
```

`prev` 是上一轮的 `(段码, 已选词, 是否手动确认过)`，重新分段时**段码没变就沿用**——
否则用户选完第一段再往下打，选择会被冲掉。

老逻辑是「整体上屏 + 键交给应用」，长句打到一半屏幕上就凭空出现半句话，
后面那个字母还会被当普通字符插进文档。

标点走 `_ =>` 分支上屏，这是主流输入法的通行做法，也是长句的自然结束方式。
但**串里有一段选不出字**（音节不像拼音，多半是在打英文）时改走上屏字母——
否则 `mmap，` 会变成「面ap，」。

## 分词模式（长句逐段选词）

`split_segments_units(units, has)` 的 DP 目标不是「切得最长」，而是**每一段都能出候选**。
纯贪心会把 `vhsjli` 切成 `vh`(张)+`sj`(三)+`li`(李)，因为 `vh` 单独就有候选。
先算 `ok[i]`（从第 i 个音节起能否切完），再在可行前提下贪心取最长段 → `vhsj`+`li`。

进入分词模式后：
- `composition_text()` 用 `[当前段]` 标光标位置，如 `[张三]李四`
- `refresh()` 在分词态下只查**当前段**的候选，并把选中项对齐到该段已选的词
- `←`/`→` 移段（`seg_idx`），`1-9` 换词（`confirm_segment`），空格确认并前移，
  回车原样上屏**字母**（`commit_raw_segments`），退格拆词（见下节）
- 编码键：**延长编码、重新分段，不上屏**（`prev` 段码未变则保留已选词）
- 其他键（标点、Tab 等）：上屏（有选不出字的段时上屏字母），键再交给应用

### 回车 = 字母，不是候选

早期分词模式把回车做成「整体上屏所有段的默认候选」，于是**同一个键的意思取决于
你敲的字母能不能切成音节**：打 `mmap`（`mm`+`ap`）回车出「面ap」，打 `nihao` 回车出
「你好」。用户没法预判，而 `mmap` 这种英文/代码串又恰好天天要打。

现在两个模式的回车是同一个语义（`commit_raw_segments` / `raw_text`）：
**上屏你敲的字母**。唯一例外是**手动确认过**的段（空格 / 数字）——那几段保留汉字，
否则「选好两个字再回车」会连人选的字一起变成字母。

想在分词模式里整句上屏中文：一路按空格确认到底，最后一段会自动整体上屏。

配套一处：`Segment.picked` 记录「用户是否手动确认过这一段」，`confirm_segment` 置位，
重新分段时随码一起沿用（`prev` 三元组）。

### 退格拆词（`segment_backspace`）

自动分段的依据只是「词库里能匹配上的最长段」，它不认识用户想要什么。
用户打 `limkyr`（李明远）时 DP 给出 `李明|远`，可他想要「李」+「明」+「远」——
这时候必须有一把手能把自动组出来的词拆回去，否则「词库里没有的词」就永远打不出来。

退格分两层，**第一层不丢编码**：

1. **拆段**：把某一段按音节拆成多个单音节段，光标落到拆出来的第一段。
   候选窗立刻变成这一段的字候选，选完自动跳到下一个字。
   - 拆哪一段由 `pick_expand_target` 决定：当前段优先；当前段本来就是单字时，
     取**最后一个**还能拆的段——退格是从右往左，用户想改的是刚打出来的那个词。
   - 拆不动（每段都是单字）才进第 2 层。
2. **退音节**：`exit_segment_mode` → `drop_last_syllable` 去掉末尾一个完整音节，
   回到「整串有候选」的普通态。

> 为什么按音节退而不是按字符退：双拼一个音节 2 键，只删一个键会停在 `tsirl`
> 这种半截串上——屏幕上一堆没意义的字母，用户还得再按一次才回到 `tsir`。
> `drop_last_syllable` 里末尾是分隔符时只删分隔符（那是用户上一步刚敲的键），
> 切不出音节时退一个字符，保证退格永远有反应。

拆段/退音节都是纯函数（`expand_segment` / `pick_expand_target` /
`drop_last_syllable`），单测直接钉住 `limkyr` 这个用例。

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
# → dist\Feihe-Setup-0.2.0.exe（约 1.25 MB，内嵌 5.8 MB 的双架构 DLL）
```

设计取舍：

1. **注册逻辑不重写**。COM / CTF TIP / 用户键盘列表全部交给 `install_tip.ps1` /
   `uninstall_tip.ps1` —— 它们是唯一真相，`register.cmd` 用的也是同一份。安装包只负责
   「提权 + 拷文件 + 调脚本 + 卸载时反向调」。Pascal 里再写一套必然漂移。
2. **装到 `%ProgramFiles%\Feihe`**，不是老的 `%ProgramData%\Feihe`。安装时尽力清理老副本
   （`Delete /REBOOTOK` 处理被占用的）。
3. **发布模式构建**（`FEIHE_NO_USER_DICT=1`）。`src/user_dict.txt` 是 5 万条个人打字记录，
   普通构建会并进 `base.bin`（86749 条 → 38024 条，DLL 7.2 MB → 3.0 MB）。

六个坑（按踩到的顺序）：

- **`.nsi` 必须带 UTF-8 BOM**。NSIS 只在文件开头有 BOM 时才按 UTF-8 解析，否则按系统 ACP
  读。中文注释直接报 `Bad text encoding: feihe.nsi:1` + `Error in script "feihe.nsi" on
  line 1`，报错完全指不到真因。`build_setup.ps1` 现在编译前自动补 BOM。
- **`$ErrorActionPreference = 'Stop'` 会把子进程 stderr 变成终止性错误**。`build.rs` 的
  `cargo:warning=base.bin: N 条` 走 stderr，于是「改了词库」这种再正常不过的情况会让
  整个构建红屏挂掉，报 `NativeCommandError` + `FullyQualifiedErrorId`，看着像编译失败，
  其实 cargo 早就成功了。**光写 `2>&1` 收进变量拦不住**——PS 5.1 里 ErrorRecord 在产生的
  那一刻就触发了 `Stop`，赋值语句根本执行不到。唯一可靠的写法是临时降级：
  ```powershell
  $prev = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
  try { $out = & $Exe @ArgList 2>&1; $code = $LASTEXITCODE }
  finally { $ErrorActionPreference = $prev }
  ```
  `build.ps1` 的 `Invoke-Cargo` 和 `build_setup.ps1` 的 `Invoke-Native` 都是这个形状。
  顺带：`makensis -V3` 也走这条路，以前只是恰好没往 stderr 写过东西。
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
| 换 DLL 不用注销 | 改名被占用的旧 DLL → 拷新的 → install_tip.ps1 末尾杀 ctfmon（Windows 按需自动重启，中等完整性；从提权进程手动拉会因 UIPI 瘫痪）→ 新 ctfmon 按注册表路径加载新 DLL，点一下输入框即生效 |
| `.ps1` 无 BOM | PowerShell 5.1 读无 BOM 脚本用系统 ANSI 代码页（简中 = cp936），UTF-8 中文按 GBK 解码会吞掉字符串闭合引号 → 解析失败（`字符串缺少终止符`）。uninstall_tip.ps1 曾中招：头部写着「纯 ASCII」实际含中文，卸载跑过了注册还在。全仓 `.ps1` 现在都带 BOM |
| `to_interface()` | 只在 `#[implement]` 生成的 `_Impl` 类型上，普通结构体上没有 → impl 块要拆开 |
| `TF_SELECTION.range` | 是 `ManuallyDrop<Option<ITfRange>>`，数组析构不释放 → 用完要手动 `take()` |
| 搜狗词库编码 | 明文 **UTF-16LE**，不是 UTF-8；文件头魔数 `SGPU`（usr）/ `SGBG`（usrbg） |
| 搜狗词条结构 | usrbg：`[u16 blen][text][u16 blen+6]`，尾部字段是天然校验和 |
| 搜狗词库位置 | `HKLM\SOFTWARE\WOW6432Node\SogouInput` 默认值 = 安装目录，用户数据在 `<安装>/用户数据/SogouPY.users/<id>/` |
| 词库合并 | `sort_by` 是稳定排序 → 先插用户词再插基础词，同码时用户词自然在前 |
| 词库性能 | 首次查询含全量解析约 11.5ms，后续 584ns |
| **UILess 两套常量** | 宿主传 `TF_TMAE_UIELEMENTENABLEDONLY` = **4**（msctf.h），TIP 收 `TF_AE_UIELEMENTENABLEDONLY` = **1**（tsf.h）。宿主若传 1，实际是 `TF_TMAE_NOACTIVATETIP`：TIP 被推迟激活、flags 丢掉，UILess 永远不生效，**不报错** |
| **UILess 前置分类** | 宿主以 UILess 方式激活时，MSCTF **只加载**注册了 `{49D2F9CF-1F5E-11D7-A6D3-00065B84435C}`（`GUID_TFCAT_TIPCAP_UIELEMENTENABLED`）的 TIP。缺这条，游戏里的症状不是「候选看不见」而是**输入法整个不出现**，`ActivateEx` 照样返回 S_OK |
| **TIP 常被提前激活** | user32 的 IMM32 兼容层会在**建窗口 / 拿焦点**时用 `dwflags = 0` 把 TIP 拉起来，宿主的 `ActivateEx` 通常晚一步。所以 TIP 判 UILess 必须用 `ITfThreadMgrEx::GetActiveFlags()` 兜底（实测带 UILess 时为 `0x80000004`） |
| `ITfDocumentMgr::CreateContext` | windows-rs 0.62 的签名**多一个 `dwFlags`**，和 tsf.h 不一致，但**照它调是对的**（传 0）。自己按 4 参数 transmute 会直接崩 |
| `ITfUIElementSink::BeginUIElement` | 回调只有 `(dwID, *mut BOOL)`，**没有元素对象**；要拿候选得回头 `ITfUIElementMgr::GetUIElement(id)` 再 QI `ITfCandidateListUIElement` |
| `ITfUIElementSink` 注册 | 通过 `ITfSource::AdviseSink(IID_ITfUIElementSink, ...)`（`ITfUIElementMgr` 自己没有 Advise 方法）。IID 直接用 `<ITfUIElementSink as Interface>::IID` |

## 调试手段

**诊断日志已经删掉了。** 早期有一份 `src/log.rs`，往 `%TEMP%\feihe-trace.log` 打
`[毫秒][pid][ThreadId] 内容`，在 `DllGetClassObject` / `CreateInstance` / `Activate` /
`OnTestKeyDown` / `OnKeyDown` / `DoEditSession` / `candidate::show` 上都打了点。
验收完成后连同 8 个文件里 55 处 `crate::trace!` 一起清了——发给别人的包不该往
`%TEMP%` 写东西，而且每次按键都开文件句柄本身就有成本。

要重新抓现场就照这个顺序恢复：把 `log.rs` 加回 `lib.rs` 的 `mod`，
`trace!` 写成 `macro_rules!` 空实现即可（调用点已删，得按上面列表重新插）。

**排查 TSF 问题的正确顺序**：先确认 sink 到底有没有被调用，再看组合态，最后才看候选窗。
不要一上来就怀疑 UI。

## UILess 测试宿主（`tools/tsfhost`）

独立的 cargo 包（有自己的 `[workspace]`，不参与主包构建），556 KB，只有 windows-rs 一个依赖。
它是一个**最小 TSF 宿主**：自己建窗口、建文档、转发按键、在 `BeginUIElement` 里回
`FALSE`（=「候选我来画」），然后在窗口底部画候选条。UILess 的宿主那一半能测到的，
这里基本都覆盖了。

```powershell
cd tools\tsfhost
cargo build --release --target x86_64-pc-windows-gnullvm
# 需要 llvm-mingw 在 PATH（同主项目 build.ps1）

tsfhost --selftest      # 自动敲 limkyr，打印结果后退出（0=PASS 2=FAIL）
tsfhost                 # 手动试：看得到候选条、打字能上屏才算通
tsfhost --normal        # 对照组：flags=0，TIP 自己弹候选窗
tsfhost --register      # 给飞鹤补注册 UIELEMENTENABLED 分类（管理员）
```

判据非常直接：窗口状态行里有 `Begin=` / `Update=` 计数。**只有 TIP 真的走了
UILess 通道，宿主才会收到 `BeginUIElement`**；走自绘窗时这三个数恒为 0，而屏幕上
会出现飞鹤自己的候选框。

`--selftest` 用 `keybd_event` 注入真按键（不是 `PostMessage`）——TIP 在按键路径上会读
`GetKeyState` 判修饰键，伪造消息容易测出「假通过」。

### 踩出来的三个坑（都已修）

1. **宿主常量 4 ≠ TIP 常量 1**。第一次把 `TF_AE_UIELEMENTENABLEDONLY`(1) 当宿主 flags
   传，`GetActiveFlags` 得到 `0x80000004` 之外的怪值、TIP 永远拿到 0——1 其实是
   `TF_TMAE_NOACTIVATETIP`。
2. **`ActivateEx` 常常晚于实际加载**。窗口一建、焦点一给，IMM32 就用 `dwflags = 0`
   把 TIP 拉起来了；之后再 `ActivateEx(4)` 只改线程管理器的标志，TIP 毫不知情。
   所以 `textservice.rs` 里改成「`dwflags` 或 `GetActiveFlags` 任一命中就算 UILess」。
3. **缺 UIELEMENTENABLED 分类 = 输入法不加载**。`--normal` 下一切正常、`--selftest`
   下连 `feihe.dll` 都进不来，最后靠 `ITfCategoryMgr::EnumItemsInCategory` 才看出来。
   `install_tip.ps1` 已补上该分类。

### 还没验证的

加分类需要管理员（`tsfhost --register` 非管理员返回 `E_FAIL`）。当前机器上这条注册
尚未生效，所以 **UILess 端到端仍待真机确认**：提权注册后跑
`tsfhost --selftest`，期望 `BeginUIElement > 0` 且候选列表非空。

## 待办

> 2026-10-23 逐条核实过代码，下面每条都确认「确实没做」。已做的都挪到「已完成」了。

**M3 剩余部分**（M3 列了四项，Shift 中英切换 / 点击选词 / 候选数衰减都已完成）：

- [ ] 组合串下划线（`GUID_DISPLAY_FEIHE` 已在 `clsid.rs` 定义，但没实现 `ITfDisplayAttributeProvider`）

**功能**

- [ ] `` ` `` 后的自定义短语（农历 / 干支 / 节气已完成，见「农历 / 干支 / 节气」一节）
- [ ] **阶段二**：用户词库换 SQLite（`winsqlite3.dll` + 手写 FFI），支持**删词**
      （词频学习用 `freq.rs`、置顶用 `pin.rs` 都已经做了；SQLite 现在只剩「删词 /
      事务 / 单文件一致性」这点价值，别再拿「支持词频学习」「支持置顶」当迁移理由）
- [ ] 模糊音（只在 README 的 librime 说明里提过一句，没排期。真要做建议在
      `shuangpin::encode_pinyin` 之后加一层等价类归并，而不是改词库）
- [ ] **M5（v2）**：语音输入热键 + ASR + AI 修正，完全没动

**工程债**

- [ ] 设置窗改双拼方案后，**其它已打开的进程**要重新激活才生效（方案缓存在各自的原子变量里；
      `Activate` 里已经会重读 config，所以只是「不即时」，不是「不生效」）
- [ ] `C:\ProgramData\Feihe\` 下 **4 个** `feihe.dll.old-*`（约 16MB）被运行中的进程占用，
      重启后可删（下次跑安装包也会顺手清掉：ProgramData 清理发生在 install_tip.ps1
      重启 ctfmon 之后，映射一释放就删得动）

已完成（别再当待办）：

- [x] 快捷输入农历 / 干支四柱 / 下一个节气（`tyme4rs`，见「农历 / 干支 / 节气」一节）
- [x] 候选窗鼠标点击选词（`layout()` 统一几何 + `SetCapture` 按下/抬起判定，见专节）
- [x] 单键候选数衰减（`candidate_limit`：1 键 18 / 2 键 45 / 3 键以上 90）
- [x] 删除诊断日志（`src/log.rs` + 55 处 `crate::trace!` 全清，运行期不写 `%TEMP%`）
- [x] Shift 单击切中英（`OnKeyUp` + 物理键态，见「中英切换」）
- [x] 全拼兜底（`config::pinyin_mode()` + 独立的全拼索引）
- [x] 回车原样上屏编码（`commit_raw`，快捷输入例外）
- [x] 整码优先（原 `exact_end` 两段查；现已并入 `top_by_rank` 的 `(码长, 行号)` 排序）
- [x] 一级简码（`brief.rs`，26 键一简字；顺带修好「单键只出一个音节」的老毛病）
- [x] 手动置顶（`pin.rs` + `Ctrl+数字`，压过一简字和动态词频）
- [x] 不弹窗模式（`config::no_popup()`，候选塞进组合串 + `Ctrl+Shift+U` 切回）
- [x] 自造词（`phrase.rs` + `compose_phrase`，分词上屏时记住用户拼出来的新词）
- [x] TSF UILess（`src/uielement.rs`，`ITfCandidateListUIElement` + `BeginUIElement` 协商）
- [x] 分词模式（`maybe_split` + `try_segment` + `segments`，**不自动上屏**）
- [x] 全拼智能分隔（`split_pinyin` 的 DP + `is_syllable`）
- [x] `;` 手动分隔（微软双拼改用 `'`）
- [x] 32 位 DLL（`i686-pc-windows-gnullvm` + Wow6432Node 双份注册）
- [x] NSIS 安装包（`installer\feihe.nsi` + `build_setup.ps1`，发布模式构建）
- [x] 用户词库编译成 `user.bin` + mmap（`dictenc` 单一编码器 + 设置窗按钮 + 导入后自动编译）

## 已知问题

- `sgim_gd_usr.bin` 的宽松提取会带少量噪声词（字节错位产物），只在恰好打到对应码时出现
