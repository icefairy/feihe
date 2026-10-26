//! 飞鹤输入法 · TSF 文本服务核心
//!
//! 链路：按键 sink → 双拼码累积 → 词库前缀查候选 → 组合串（composition）显示码
//! → 空格/数字选词 → 提交候选，结束 composition。
//!
//! 关键点：
//! - 所有文档写操作都在 edit session 内完成（TSF 硬性要求）。
//! - 组合串走 `ITfContextComposition::StartComposition`，而不是裸 `SetText`。
//!   这样 TSF 才知道这是"待编辑文本"，宿主应用和系统都能正确处理（撤销、光标、
//!   输入法状态），也是后面加下划线样式的前提。
//! - 按键默认放行：只有组合态或字母键才接管，Ctrl/Alt/Win 组合一律放行。

use crate::assist::AssistMode;
use crate::candidate;
use crate::config;
use crate::dict;
use crate::dllcount;
use crate::insert;
use crate::quick;
use crate::settings;
use crate::shuangpin::{self, Scheme};
use crate::softkbd;
use crate::statusbar;
use crate::uielement;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Variant::{VariantInit, VT_I4};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_OEM_1, VK_OEM_2, VK_OEM_4, VK_OEM_5,
    VK_OEM_6, VK_OEM_7, VK_OEM_COMMA, VK_OEM_MINUS, VK_OEM_PERIOD, VK_OEM_PLUS, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::TextServices::*;
use windows_core::{implement, IUnknownImpl};

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

/// 一次最多取多少候选（10 页 × 9）。单键 / 两键会按码长衰减，见 `candidate_limit`。
const MAX_CANDIDATES: usize = 90;

/// 候选上限按码长衰减。
///
/// 敲一个 `n` 能查出 90 个候选 = 10 页，第 4 页往后基本是生僻字，翻过去也没用，
/// 但「一直能翻」这件事本身就会让人怀疑是不是翻不完。所以：
///
/// - 1 键：18 个（2 页）
/// - 2 键：45 个（5 页）
/// - 3 键及以上：满额 90
///
/// 按**去掉手动分隔符和空白后的字符数**算，`ni'hao` 和 `nihao` 拿同样的额度。
fn candidate_limit(code: &str) -> usize {
    let n = code
        .chars()
        .filter(|c| *c != FeiheTextService::SEP && !c.is_whitespace())
        .count();
    match n {
        0 | 1 => 18,
        2 => 45,
        _ => MAX_CANDIDATES,
    }
}

enum EditOp {
    /// 建立 composition 并写入首个双拼码。
    /// 文本由 `begin_composition` 现场从 `composition_text()` 取，字段只是占位。
    Start,
    /// 更新 composition 文本（继续敲键 / 退格）
    Update(String),
    /// 用最终文本替换组合串，结束 composition
    Commit(String),
    /// 不建 composition，直接在光标处插入文本（全角标点、软键盘符号）
    Insert(String),
    /// 清空组合串，结束 composition
    Cancel,
}

/// 分词模式里的一段
#[derive(Clone, Debug)]
struct Segment {
    /// 这一段的编码（如 `vhsj`）
    code: String,
    /// 这一段当前选中的词（未确认时是第一个候选）
    word: String,
    /// 用户是否用空格/数字**手动确认过**这一段。
    ///
    /// 回车是「我就要这些字母」，但用户已经逐段选出来的字不能跟着丢掉，
    /// 所以上屏字母时 `picked` 的段保留汉字，其余段输出原始编码。
    picked: bool,
}

#[implement(
    ITfTextInputProcessorEx,
    ITfKeyEventSink,
    ITfEditSession,
    ITfCompositionSink
)]
pub struct FeiheTextService {
    tid: Cell<u32>,
    thread_mgr: RefCell<Option<ITfThreadMgr>>,
    context: RefCell<Option<ITfContext>>,
    composition: RefCell<Option<ITfComposition>>,
    /// 已输入的双拼码
    code: RefCell<String>,
    /// 已输入的辅助码（Tab 之后输入的键）
    assist: RefCell<String>,
    /// 是否处于辅助码输入态
    assist_active: Cell<bool>,
    /// 本次输入里用户已经拒绝过一次「3 键直出」，别再拆了。
    ///
    /// 没有这个位，开了开关就**永远打不出「代价」**：`dd`+`j` 被自动拆成辅助码，
    /// 退格回到 `dd` 后再打 `j` 又被拆一次，死循环。拒绝一次就放过，直到这次
    /// 组合结束（上屏 / Esc）——用户真想用辅助码还有 `Tab`。
    assist_direct_denied: Cell<bool>,
    /// 当前候选，已按词频降序
    candidates: RefCell<Vec<Cow<'static, str>>>,
    /// 当前页内选中索引
    selected: Cell<usize>,
    /// 当前页号
    page: Cell<usize>,
    /// 上一次算出的光标位置（屏幕坐标），翻页/移动选择时复用
    caret_x: Cell<i32>,
    caret_y: Cell<i32>,
    pending_op: RefCell<Option<EditOp>>,
    /// Shift 按住期间有没有按过别的键。用来区分「单击 Shift」和「Shift+字母」：
    /// 单击切中英，组合键照旧交给应用（大写字母、Shift 选择都靠它）。
    shift_used: Cell<bool>,
    /// 分词模式的段列表。空 Vec 表示不在分词模式。
    segments: RefCell<Vec<Segment>>,
    /// 分词模式下光标落在第几段
    seg_idx: Cell<usize>,
    /// 宿主是不是 UI-less 线程（独占全屏游戏）。由 `ActivateEx` 的 `dwflags` 决定。
    ///
    /// 置位后候选走 `ITfUIElementMgr` 交给宿主画，我们自己一个窗口都不弹。
    /// 宿主不接（`BeginUIElement` 回 TRUE）就降级回自绘窗，这个位会清掉。
    ui_less: Cell<bool>,
    /// UILess 会话。激活时建一次，反激活时销毁。
    ///
    /// 存 `Rc` 是为了能用「克隆一份句柄再调用」的方式用：`update` 会同步回调进
    /// 宿主的 sink，宿主要是再问我们要候选（`GetString`），`RefCell` 上还会多一层
    /// 借用——万一重入就是 panic。拿句柄、放借用、再调，绕开这个坑。
    uielement: RefCell<Option<Rc<uielement::Session>>>,
    /// 存活令牌。候选窗的点击回调拿着它的 `Weak`——对象析构后 `upgrade()` 失败，
    /// 回调就不会去碰已经释放的地址。见 `install_click_handler`。
    alive: Arc<()>,
}

impl FeiheTextService {
    pub fn new() -> Self {
        dllcount::object_created();
        Self {
            tid: Cell::new(0),
            thread_mgr: RefCell::new(None),
            context: RefCell::new(None),
            composition: RefCell::new(None),
            code: RefCell::new(String::new()),
            assist: RefCell::new(String::new()),
            assist_active: Cell::new(false),
            assist_direct_denied: Cell::new(false),
            candidates: RefCell::new(Vec::new()),
            selected: Cell::new(0),
            page: Cell::new(0),
            caret_x: Cell::new(0),
            caret_y: Cell::new(0),
            pending_op: RefCell::new(None),
            shift_used: Cell::new(false),
            segments: RefCell::new(Vec::new()),
            seg_idx: Cell::new(0),
            ui_less: Cell::new(false),
            uielement: RefCell::new(None),
            alive: Arc::new(()),
        }
    }

    fn is_composing(&self) -> bool {
        !self.code.borrow().is_empty()
    }

    /// 清掉输入状态（不动文档，也不动 composition 引用）
    fn clear_state(&self) {
        self.code.borrow_mut().clear();
        self.assist.borrow_mut().clear();
        self.assist_active.set(false);
        self.assist_direct_denied.set(false);
        self.segments.borrow_mut().clear();
        self.seg_idx.set(0);
        self.candidates.borrow_mut().clear();
        self.selected.set(0);
        self.page.set(0);
        self.pending_op.borrow_mut().take();
        self.hide_candidates();
    }

    /// 收掉候选显示。自绘窗和 UILess 元素都要收——只收一边的话，提交后宿主的
    /// 候选框会一直挂在屏幕上（它不知道我们这边组合已经结束了）。
    fn hide_candidates(&self) {
        candidate::hide();
        // 先把句柄克隆出来再调用，别让 RefCell 的借用跨过 COM 回调
        let sess = self.uielement.borrow().as_ref().cloned();
        if let Some(s) = sess {
            s.end();
        }
    }

    fn deactivate_sink(&self) {
        if let Some(tim) = self.thread_mgr.borrow().as_ref() {
            if let Ok(mgr) = tim.cast::<ITfKeystrokeMgr>() {
                unsafe {
                    let _ = mgr.UnadviseKeyEventSink(self.tid.get());
                }
            }
        }
        self.thread_mgr.borrow_mut().take();
        self.context.borrow_mut().take();
        self.composition.borrow_mut().take();
        self.clear_state();
        // UILess 会话握着 ITfThreadMgr 的派生接口，反激活时必须先放掉，
        // 否则 MSCTF 会在线程管理器已经失效之后还来问候选数据。
        self.uielement.borrow_mut().take();
        self.ui_less.set(false);
        candidate::destroy();
    }

    fn modifiers_down() -> bool {
        unsafe {
            let down = |vk: u16| (GetKeyState(vk as i32) as u16) & 0x8000 != 0;
            down(VK_CONTROL.0) || down(VK_MENU.0) || down(VK_LWIN.0) || down(VK_RWIN.0)
        }
    }

    fn shift_down() -> bool {
        unsafe { (GetKeyState(VK_SHIFT.0 as i32) as u16) & 0x8000 != 0 }
    }

    /// 只按了 Ctrl（不含 Alt / Win）。
    ///
    /// `Ctrl+数字` 是「置顶当前候选」，和 `Ctrl+A` 那类系统快捷键不是一回事，
    /// 所以单独判一次——`modifiers_down` 分不出是哪个修饰键。
    fn ctrl_only() -> bool {
        unsafe {
            let down = |vk: u16| (GetKeyState(vk as i32) as u16) & 0x8000 != 0;
            down(VK_CONTROL.0) && !down(VK_MENU.0) && !down(VK_LWIN.0) && !down(VK_RWIN.0)
        }
    }

    /// 手动分隔符。内部统一存成 `'`，组合串里也原样显示。
    const SEP: char = '\'';

    /// 这个虚拟键是不是「手动分隔」键。
    ///
    /// 全拼和小鹤双拼用分号键（`;`）——它们的编码里没有分号；
    /// 微软双拼的分号被 `ing` 韵母占用，只能用单引号键顶替，否则 `ing` 打不出来。
    fn is_separator_key(vk: u8) -> bool {
        if config::pinyin_mode() || dict::scheme() != Scheme::Ms {
            vk == VK_OEM_1.0 as u8
        } else {
            vk == VK_OEM_7.0 as u8
        }
    }

    /// 引导符本身当字符上屏时的样子。开了全角标点就给全角 `／`，和 `，` `。` 一致。
    ///
    /// 抽成纯函数是为了能单测——`commit_raw` 要 `&self`，测试里造不出来。
    fn prefix_literal(fullwidth: bool) -> char {
        if fullwidth {
            '／'
        } else {
            '/'
        }
    }

    /// 去掉手动分隔符，得到真正拿去查词库的编码。
    ///
    /// 分隔符只影响**怎么切**，不影响**查什么**：`nihao'ma` 和 `nihaoma`
    /// 查的是同一批候选，只是前者强制在 `hao` 后面断一刀。
    fn query_code(code: &str) -> Cow<'_, str> {
        if code.contains(Self::SEP) {
            Cow::Owned(code.replace(Self::SEP, ""))
        } else {
            Cow::Borrowed(code)
        }
    }

    /// 按当前模式查词库（全拼走全拼索引，双拼走双拼索引）。
    ///
    /// 分词 / 拆分上屏都要「拿一小段码试一下有没有候选」，两种模式的索引不同，
    /// 所以统一收口到这里，避免每个调用点都写一遍 `if pinyin_mode()`。
    fn lookup_any(code: &str, limit: usize) -> Vec<Cow<'static, str>> {
        if config::pinyin_mode() {
            dict::lookup_pinyin(code, limit)
        } else {
            dict::lookup(code, limit)
        }
    }

    /// 带辅助码查一次，只关心「筛出东西没有」。
    fn assist_hit(prefix: &str, assist: &str, mode: AssistMode) -> bool {
        if prefix.is_empty() || assist.is_empty() {
            return false;
        }
        let hit = if config::pinyin_mode() {
            dict::lookup_pinyin_filtered(prefix, assist, mode, 1)
        } else {
            dict::lookup_filtered(prefix, assist, mode, 1)
        };
        !hit.is_empty()
    }

    /// 「先连打、再按 Tab」：把编码末尾的辅助码拆出来。
    ///
    /// 自然码 / 小鹤音形的习惯是**一个码打完就完**（自然码 拼音+部首、小鹤 拼音+首末形），
    /// 所以很多人会先把 `ddj` 敲完再想起辅助码。这时按 `Tab`，若末尾 1~2 键能当辅助码
    /// 用（剥掉之后还能出候选），就挪过去，组合串直接变成 `dd j`——不用退格重来。
    ///
    /// 判据里那条**前缀长度必须是偶数**是关键：双拼每个音节恰好两键，剥完还剩奇数
    /// 长度说明剥的是「半个音节」（`de` + Tab 不该把 `e` 当成辅助码），这种情况一律不拆。
    ///
    /// 全拼模式不走这套：音节长度 1~6 不定，奇偶没意义，只能靠用户手动 Tab。
    ///
    /// 抽成纯函数是为了能单测（`self` 里全是 COM 句柄，测试进程造不出来）。
    fn split_trailing_assist(
        code: &str,
        max: usize,
        mut hit: impl FnMut(&str, &str) -> bool,
    ) -> Option<(String, String)> {
        if max == 0 || code.contains(Self::SEP) {
            return None;
        }
        let bytes = code.as_bytes();
        if !bytes.iter().all(u8::is_ascii_lowercase) {
            return None;
        }
        for t in (1..=max).rev() {
            // 剩不下两个键（至少一个完整音节）就试下一个更短的上限——`ddj` + 小鹤
            // 上限 2 时，2 键试不了但 1 键能试，所以是 continue 不是 break
            if bytes.len() < t + 2 {
                continue;
            }
            let cut = bytes.len() - t;
            // 剩下半个音节就不拆：`de` 的 `e` 不是辅助码
            if cut % 2 != 0 {
                continue;
            }
            let (head, tail) = code.split_at(cut);
            if hit(head, tail) {
                return Some((head.to_string(), tail.to_string()));
            }
        }
        None
    }

    /// 「3 键直出」用的命中判定：筛出的第一项必须是**单字**。
    ///
    /// 为什么不与 `assist_hit` 共用：3 键那一下是有歧义的（那 1 键也可能是在打下一个
    /// 音节），要求「筛出单字」能把误伤压到很低——半截音节 `ni` + `h` 这种组合通常
    /// 连候选都没有，而 `dd` + 部首 `j` 的第一项恰好就是单字「带」。
    fn assist_hit_single(prefix: &str, assist: &str, mode: AssistMode) -> bool {
        if prefix.is_empty() || assist.is_empty() {
            return false;
        }
        let hit = if config::pinyin_mode() {
            dict::lookup_pinyin_filtered(prefix, assist, mode, 1)
        } else {
            dict::lookup_filtered(prefix, assist, mode, 1)
        };
        hit.first().is_some_and(|w| w.chars().count() == 1)
    }

    /// 「3 键直出」：音形模式下敲到第 3 键就把末尾 1~2 键挪进辅助码（`ddj` → `dd`+`j`）。
    ///
    /// 为什么只认 3 键：双拼每个音节恰好 2 键，所以 3 键永远是「1 个完整音节 +
    /// 1 个孤立键」；4 键可能已经是一个完整词（`ddji`），5 键可能是两个半音节，
    /// 越长的码越不该替用户猜。**3 键这一处是唯一「猜错了也还算合理」的位置。**
    ///
    /// 抽成纯函数是为了能单测（`self` 里全是 COM 句柄，测试进程造不出来）。
    fn auto_assist_split(
        code: &str,
        max: usize,
        hit: impl FnMut(&str, &str) -> bool,
    ) -> Option<(String, String)> {
        if code.len() != 3 {
            return None;
        }
        FeiheTextService::split_trailing_assist(code, max, hit)
    }

    /// 把「3 键直出」落到状态上。只动编码，候选随后由 `refresh` 统一重算。
    ///
    /// 拆完把 `assist_active` 置真，等价于用户自己按了一下 `Tab`：组合串显示成
    /// `dd j`，Tab 再按一次退回（清掉辅助码）。后续字母进辅助码而不是编码——
    /// 这是刻意的，第 4 键如果还能出词，说明用户想要的本来就不是这个开关。
    fn auto_assist(&self) {
        let cfg = config::get();
        let mode = cfg.assist_mode;
        if !cfg.assist_direct
            || mode == AssistMode::Off
            || config::pinyin_mode()
            || self.assist_active.get()
            || self.assist_direct_denied.get()
        {
            return;
        }
        let code = self.code.borrow().clone();
        let Some((head, tail)) = FeiheTextService::auto_assist_split(&code, mode.max_keys(), |p, a| {
            FeiheTextService::assist_hit_single(p, a, mode)
        }) else {
            return;
        };
        self.code.borrow_mut().clone_from(&head);
        self.assist.borrow_mut().clone_from(&tail);
        self.assist_active.set(true);
    }

    /// 把「按错读音打」的词追加到候选末尾（`zhongqing` → 重庆）。
    ///
    /// 抽成纯函数是为了能单测：`refresh()` 要 `&self`，而 `self` 里全是 COM 句柄，
    /// 测试进程造不出来。逻辑本身很薄——去重、追加、不改变已有顺序——但「追加到
    /// 末尾」这个位置是刻意的：撞码的变体里排前面的永远是用户真正想要的那个
    /// （`zhongqing` 首先是「钟情」）。
    fn append_wrong_reading(list: &mut Vec<Cow<'static, str>>, code: &str, flat: bool) {
        for w in crate::hint::wrong_reading(code, flat, 2) {
            if !list.iter().any(|x| x.as_ref() == w) {
                list.push(Cow::Borrowed(w));
            }
        }
    }

    /// 全拼串 → 音节序列。
    ///
    /// DP：`ok[i]` 表示 `s[i..]` 能否完整切成合法音节；可行时贪心取最长
    /// （`xian` 优先于 `xi`+`an`，符合「长音节优先」的直觉）。
    /// 切不出来返回 `None`——说明用户还没敲完，或者敲了不存在的拼写。
    fn split_pinyin(s: &str) -> Option<Vec<String>> {
        let chars: Vec<char> = s.chars().collect();
        let n = chars.len();
        if n == 0 {
            return None;
        }
        let max = shuangpin::MAX_SYLLABLE_LEN;
        let at = |i: usize, len: usize| -> String { chars[i..i + len].iter().collect() };

        let mut ok = vec![false; n + 1];
        ok[n] = true;
        for i in (0..n).rev() {
            for len in 1..=max.min(n - i) {
                if ok[i + len] && shuangpin::is_syllable(&at(i, len)) {
                    ok[i] = true;
                    break;
                }
            }
        }
        if !ok[0] {
            return None;
        }

        let mut out = Vec::new();
        let mut i = 0;
        while i < n {
            let mut step = 0;
            for len in (1..=max.min(n - i)).rev() {
                if ok[i + len] && shuangpin::is_syllable(&at(i, len)) {
                    step = len;
                    break;
                }
            }
            if step == 0 {
                return None;
            }
            out.push(at(i, step));
            i += step;
        }
        Some(out)
    }

    /// 双拼串 → 音节序列（定长 2）。某个分组长度是奇数说明音节没打完。
    fn syllables_ds(code: &str) -> Vec<String> {
        let mut out = Vec::new();
        for group in code.split(Self::SEP) {
            if group.is_empty() {
                continue; // 连续分隔符 / 结尾多敲了一个
            }
            let chars: Vec<char> = group.chars().collect();
            if chars.len() % 2 != 0 {
                return Vec::new();
            }
            for c in chars.chunks(2) {
                out.push(c.iter().collect());
            }
        }
        out
    }

    /// 全拼串 → 音节序列。任一分组切不出来就整体放弃。
    fn syllables_full(code: &str) -> Vec<String> {
        let mut out = Vec::new();
        for group in code.split(Self::SEP) {
            if group.is_empty() {
                continue;
            }
            match Self::split_pinyin(group) {
                Some(v) => out.extend(v),
                None => return Vec::new(),
            }
        }
        out
    }

    /// 把编码拆成音节序列，`'` 是用户强制指定的边界，一定在那里断开。
    ///
    /// 返回空 Vec 表示「还不能切」——双拼某个分组长度是奇数、或全拼某个分组
    /// 切不出合法音节。调用方据此决定再等等，而不是急着拆分上屏。
    fn syllables(code: &str) -> Vec<String> {
        if config::pinyin_mode() {
            Self::syllables_full(code)
        } else {
            Self::syllables_ds(code)
        }
    }

    /// 把第 `idx` 段拆成逐音节段（「李明」→「李」+「明」）。
    ///
    /// 返回 `false` 表示拆不动——本来就是单音节段。
    ///
    /// 自动分段是 DP 贪心取最长（`limkyr` → `李明|远`），但那个切法只是**词库里
    /// 最长的匹配**，不一定是用户想要的。退格把这一段拆开，用户就能一个字一个字
    /// 自己组。拆出来的每段先给第一个候选当默认值，和首次进入分词时一个口径。
    fn expand_segment(segs: &mut Vec<Segment>, idx: usize) -> bool {
        let Some(seg) = segs.get(idx) else {
            return false;
        };
        let units = FeiheTextService::syllables(&seg.code);
        if units.len() < 2 {
            return false;
        }
        let split: Vec<Segment> = units
            .iter()
            .map(|u| Segment {
                code: u.clone(),
                word: FeiheTextService::lookup_any(u, 1)
                    .first()
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                picked: false,
            })
            .collect();
        segs.splice(idx..=idx, split);
        true
    }

    /// 退格该拆哪一段：当前段优先，其次最后一个还能拆的段。
    ///
    /// 「最后」而不是「第一个」是因为退格的方向是从右往左——用户刚打出来的那个词
    /// 才是他想改的。全都不够拆（每段都是单字）返回 `None`，调用方去删音节。
    fn pick_expand_target(segs: &[Segment], idx: usize) -> Option<usize> {
        let splittable = |i: usize| {
            segs.get(i)
                .is_some_and(|s| FeiheTextService::syllables(&s.code).len() > 1)
        };
        if splittable(idx) {
            return Some(idx);
        }
        (0..segs.len()).rev().find(|&i| splittable(i))
    }

    /// 去掉编码末尾的一个完整音节（连同它后面的分隔符）。
    ///
    /// 双拼音节定长 2，只删一个键会留下 `tsirl` 这种半截串——屏幕上是一堆没有
    /// 意义的字母，用户还得再按一次才回到 `tsir`。按音节退，一次退到上一个
    /// 能出候选的位置。
    ///
    /// 切不出音节（还没打完）时退一个字符，保证退格永远有反应。
    fn drop_last_syllable(code: &str) -> String {
        // 末尾是分隔符就先删它——那是用户上一步刚敲的键，不该连音节一起吞掉
        if let Some(s) = code.strip_suffix(Self::SEP) {
            return s.to_string();
        }
        let mut s = code.to_string();
        let units = FeiheTextService::syllables(&s);
        let n = units
            .last()
            .map(|u| u.chars().count())
            .unwrap_or(1)
            .max(1);
        for _ in 0..n {
            if s.pop().is_none() {
                break;
            }
        }
        s.trim_end_matches(Self::SEP).to_string()
    }

    /// 把分段选择拼成上屏文本。纯函数，理由同 `plan_segments`。
    ///
    /// 空段必须兜底：`plan_segments` 退到逐音节时，个别音节可能查不到候选，
    /// 直接拼 `word` 会把它**静默吞掉**——用户敲了 3 个音节只得到 2 个字，
    /// 而且屏幕上没有任何提示。先按段码再查一次，仍然没有就上屏段码本身
    /// （和 `commit_raw` 一个口径：宁可让用户看见自己敲了什么）。
    /// 分词模式下「原样上屏」要上屏什么：确认过的段出汉字，其余段出原始编码。
    ///
    /// 纯函数，好单测。`mmap` 这种串会被切成 `mm`+`ap`，`ap` 没有候选（`word` 为空），
    /// 于是整串回落成 `mmap`；用户要是先按空格确认了「面」，那就是 `面ap`——
    /// 他自己选的字不该被吃掉。
    fn raw_text(segs: &[Segment]) -> String {
        let mut s = String::new();
        for seg in segs {
            if seg.picked && !seg.word.is_empty() {
                s.push_str(&seg.word);
            } else {
                s.push_str(&seg.code);
            }
        }
        s
    }

    fn merge_segments(segs: &[Segment]) -> String {
        segs.iter()
            .map(|s| {
                if !s.word.is_empty() {
                    return s.word.clone();
                }
                FeiheTextService::lookup_any(&s.code, 1)
                    .first()
                    .map(|w| w.to_string())
                    .unwrap_or_else(|| s.code.clone())
            })
            .collect()
    }

    /// 从一次分词上屏里推导出「该记成自造词的新词」。
    ///
    /// 纯函数，`lookup` 是「码 + 词 → 全拼」的反查（生产路径传 `dict::pinyin_of`，
    /// 测试传假数据）。返回 `None` 表示这次不该记——下面每条都是为了让表里
    /// 只留「用户真的一个一个字点出来的新词」：
    ///
    /// 1. 至少两段：单字不叫组词，单字的排序已经由动态词频管了
    /// 2. 每段都真有词：空段是 `merge_segments` 兜底时塞的段码，不是用户选的
    /// 3. 段码拼起来等于用户敲的那串：中途改过编码的话，码和词已经对不上了
    /// 4. 词库里本来就有：自动切分沿用默认候选时不该记重复数据
    ///
    /// 全拼**任何一段查不到就整体放弃**：拿不到可靠的全拼，这个词在换方案、
    /// 开全拼模式之后就再也打不出来，记了反而更糟（用户以为记下了）。
    fn compose_phrase(
        segs: &[Segment],
        code: &str,
        lookup: impl Fn(&str, &str) -> Option<String>,
    ) -> Option<(String, String)> {
        if segs.len() < 2 {
            return None;
        }
        if segs.iter().any(|s| s.word.is_empty() || s.word.is_ascii()) {
            return None;
        }
        if segs.iter().map(|s| s.code.as_str()).collect::<String>() != code {
            return None;
        }
        let word: String = segs.iter().map(|s| s.word.as_str()).collect();
        if lookup(code, &word).is_some() {
            return None;
        }
        let mut py = String::new();
        for s in segs {
            let p = lookup(&s.code, &s.word)?;
            if !py.is_empty() {
                py.push(' ');
            }
            py.push_str(&p);
        }
        Some((word, py))
    }

    /// 把编码切成若干段。纯函数，不碰 `self`——分段策略是长句输入的核心决策，
    /// 必须能单测（`_Impl` 在 windows-rs 里构造不出来，没法拿实例测）。
    ///
    /// 先按「每段都能出候选」做 DP；DP 失败（某个音节词库里完全没有）就退到逐音节。
    /// 返回空 Vec 表示音节不完整（还没打完），调用方应「再等等」。
    fn plan_segments(code: &str, has: impl Fn(&str) -> bool) -> Vec<String> {
        let units = FeiheTextService::syllables(code);
        if units.len() < 2 {
            return Vec::new();
        }
        let codes = FeiheTextService::split_segments_units(&units, has);
        if codes.len() >= 2 {
            codes
        } else {
            units
        }
    }

    /// 把音节序列切成多段，要求**每一段都能出候选**。
    /// 返回空 Vec 表示无法完整切分（调用方应退回别的策略）。
    ///
    /// 用 DP 而不是贪心最长：`vhsjli` 纯贪心会切成 `vh`(张) + `sj`(三) + `li`(李)，
    /// 因为 `vh` 单独就有候选。先算 `ok[i]`（从第 i 个音节起能否切完），
    /// 再在可行的前提下贪心取最长段，得到 `vhsj`(张三) + `li`(李)。
    fn split_segments_units(units: &[String], has: impl Fn(&str) -> bool) -> Vec<String> {
        let n = units.len();
        if n < 2 {
            return Vec::new();
        }
        let join = |i: usize, len: usize| -> String { units[i..i + len].concat() };

        // ok[i] = 从第 i 个音节到末尾可以完整切分
        let mut ok = vec![false; n + 1];
        ok[n] = true;
        for i in (0..n).rev() {
            for len in 1..=(n - i) {
                if ok[i + len] && has(&join(i, len)) {
                    ok[i] = true;
                    break;
                }
            }
        }
        if !ok[0] {
            return Vec::new();
        }

        // 贪心取最长段（在 ok 约束下）
        let mut out = Vec::new();
        let mut i = 0;
        while i < n {
            let mut step = 0;
            for len in (1..=(n - i)).rev() {
                if ok[i + len] && has(&join(i, len)) {
                    step = len;
                    break;
                }
            }
            if step == 0 {
                break;
            }
            out.push(join(i, step));
            i += step;
        }
        out
    }

    /// 按当前模式切段（生产路径直接用 `split_segments_units`，因为它已经拿着
    /// `units`；这个包装只给测试和排查用）
    #[cfg(test)]
    fn split_segments(code: &str) -> Vec<String> {
        let units = Self::syllables(code);
        Self::split_segments_units(&units, |p| !Self::lookup_any(p, 1).is_empty())
    }

    /// 是不是 Shift 键本身。宿主可能报 VK_SHIFT(0x10)，也可能报左右键(0xA0/0xA1)。
    fn is_shift(vk: u8) -> bool {
        vk == VK_SHIFT.0 as u8 || vk == 0xA0 || vk == 0xA1
    }

    /// 全角标点映射（只处理不带 Shift 的那一排键）。
    /// Shift 组合（`?` `:` `|` 等）交给应用，避免和我们自己算的字符对不上。
    ///
    /// `\` → `、` 是照搜狗的习惯。`/` **不再**出顿号——它是快捷输入的引导符
    /// （见 `quick::PREFIX`），顿号留 `\` 一个键就够，两拨用户的习惯里 `\` 是
    /// 更主流的那个。
    fn fullwidth_punct(vk: u8) -> Option<char> {
        let ch = match vk {
            v if v == VK_OEM_COMMA.0 as u8 => '，',
            v if v == VK_OEM_PERIOD.0 as u8 => '。',
            v if v == VK_OEM_1.0 as u8 => '；',
            v if v == VK_OEM_5.0 as u8 => '、',
            v if v == VK_OEM_4.0 as u8 => '【',
            v if v == VK_OEM_6.0 as u8 => '】',
            v if v == VK_OEM_7.0 as u8 => '’',
            v if v == VK_OEM_MINUS.0 as u8 => '－',
            v if v == VK_OEM_PLUS.0 as u8 => '＝',
            _ => return None,
        };
        Some(ch)
    }

    /// 这个键在当前配置下要不要**自己**输出全角标点。
    ///
    /// 两个门：用户开了「全角标点」、且没按 Shift（Shift 组合如 `?` `:` `|`
    /// 交给应用自己算，免得我们算的和键盘布局对不上）。
    /// 把这两个门收在一处，是因为「没在组合态」和「正在组合态」两条路都要用，
    /// 分开写必然有一边漏掉。
    fn fullwidth_punct_for(vk: u8) -> Option<char> {
        if !config::get().fullwidth || Self::shift_down() {
            return None;
        }
        Self::fullwidth_punct(vk)
    }

    /// 该虚拟键是否属于当前方案的编码键。
    /// 双拼：小鹤只有字母；微软双拼的 `ing` 落在分号键上，所以还要放行 VK_OEM_1。
    /// 全拼：只有字母。
    /// 另外都接受手动分隔键（见 `is_separator_key`）和快捷输入的引导符 `/`。
    fn is_code_key(vk: u8) -> bool {
        if vk.is_ascii_uppercase() {
            return true;
        }
        // Shift 组合不参与编码：`Shift+/` 要输出 `?`、`Shift+\` 要输出 `|`，
        // 这些都得原样交给应用。字母不受影响（上面已经返回 true 了）。
        if Self::shift_down() {
            return false;
        }
        // 快捷输入的引导符 `/`
        if vk == VK_OEM_2.0 as u8 {
            return true;
        }
        // 微软双拼的 `ing` 在分号键
        if vk == VK_OEM_1.0 as u8 && !config::pinyin_mode() && dict::scheme() == Scheme::Ms {
            return true;
        }
        // 手动分隔
        Self::is_separator_key(vk)
    }

    /// Shift+字母要直接上屏的那个大写字母。
    ///
    /// 只认 A–Z。数字和标点键（`Shift+1`→`!`、`Shift+/`→`?`）不认——那些字符
    /// 由键盘布局决定，我们按 VK 自己映射，换成非 QWERTY 布局就会算错，
    /// 还是交给应用去换算。
    fn shifted_letter(vk: u8) -> Option<char> {
        if vk.is_ascii_uppercase() {
            return Some(vk as char);
        }
        None
    }

    /// 虚拟键 → 编码字符
    fn code_char(vk: u8) -> Option<char> {
        if vk.is_ascii_uppercase() {
            return Some((vk as char).to_ascii_lowercase());
        }
        if vk == VK_OEM_2.0 as u8 {
            return Some(quick::PREFIX);
        }
        if vk == VK_OEM_1.0 as u8 && !config::pinyin_mode() && dict::scheme() == Scheme::Ms {
            return Some(';');
        }
        if Self::is_separator_key(vk) {
            return Some(Self::SEP);
        }
        None
    }

    /// 组合串里允许出现哪些字符（全拼模式放宽到纯字母，两种模式都允许手动分隔符）
    fn code_allowed(s: &str) -> bool {
        // 快捷输入：引导符（`/`，见 quick::PREFIX）+ 纯小写字母
        if let Some(rest) = s.strip_prefix(quick::PREFIX) {
            return rest.chars().all(|c| c.is_ascii_lowercase());
        }
        if config::pinyin_mode() {
            s.chars().all(|c| c.is_ascii_lowercase() || c == Self::SEP)
        } else {
            s.chars()
                .all(|c| dict::scheme().is_code_char(c) || c == Self::SEP)
        }
    }

    /// 分隔符能不能插在当前编码后面。
    ///
    /// 三种情况没有信息量，直接吞掉：编码为空、已经有分隔符结尾、快捷输入态。
    /// 双拼额外要求断在**完整音节**之后（音节定长 2），否则 `vh'sj` 这种
    /// 半截音节会让整串切不出来，反而更难用。
    fn can_insert_sep(code: &str) -> bool {
        if code.is_empty() || code.ends_with(Self::SEP) || code.starts_with(quick::PREFIX) {
            return false;
        }
        if config::pinyin_mode() {
            return true;
        }
        let tail = code.rsplit(Self::SEP).next().unwrap_or("");
        tail.chars().count() % 2 == 0
    }

    /// 单键组合态下的「一简次选」：`d;` → 打。
    ///
    /// 只认**恰好一个字母**，并且只在双拼下：全拼里一个字母后面还能跟分隔符
    /// （`xi'an` 这类），分号键不能挪作他用。双拼恰好相反——`can_insert_sep`
    /// 要求断在完整音节（2 键）之后，所以单键后面的分号本来就被吞掉，
    /// 这个键位是空的，正好接小鹤的次选键。
    fn brief_alt(code: &str) -> Option<&'static str> {
        if config::pinyin_mode() {
            return None;
        }
        crate::brief::alt_for(code)
    }

    /// 重新查候选并回到第一页第一项
    fn refresh(&self) {
        // 分词模式：候选就是当前这一段的
        if !self.segments.borrow().is_empty() {
            let idx = self.seg_idx.get().min(self.segments.borrow().len() - 1);
            let code = self.segments.borrow()[idx].code.clone();
            let word = self.segments.borrow()[idx].word.clone();
            let list = FeiheTextService::lookup_any(&code, candidate_limit(&code));
            // 选中项对齐到这一段当前选中的词，左右移动时不会跳回第一项。
            // 页号也要跟着算出来：`sel` 可能落在第二页（「鹤」在 `he` 的第 12 位），
            // 只把 `selected` 设成 12 会让高亮框跑到页外，翻页也从错的页起翻。
            let sel = list.iter().position(|w| **w == word).unwrap_or(0);
            self.candidates.borrow_mut().clone_from(&list);
            self.selected.set(sel % candidate::PAGE_SIZE);
            self.page.set(sel / candidate::PAGE_SIZE);
            return;
        }

        let code = self.code.borrow().clone();

        // 快捷输入：`\rq` 这类，不走词库
        if let Some(key) = code.strip_prefix(quick::PREFIX) {
            let list: Vec<Cow<'static, str>> = quick::candidates(key)
                .into_iter()
                .map(Cow::Owned)
                .collect();
            self.candidates.borrow_mut().clone_from(&list);
            self.selected.set(0);
            self.page.set(0);
            return;
        }

        // 手动分隔符只影响切分，查询时要去掉：`ni'hao` 和 `nihao` 查同一批候选
        let query = FeiheTextService::query_code(&code);
        let query: &str = query.as_ref();

        let assist_prefix = self.assist.borrow().clone();
        let mode = config::get().assist_mode;
        let limit = candidate_limit(&code);
        let mut list = if config::pinyin_mode() {
            // 全拼模式：前缀是「全拼去掉空格」的串（nihao），走另一套索引
            dict::lookup_pinyin_filtered(query, &assist_prefix, mode, limit)
        } else if assist_prefix.is_empty() {
            dict::lookup(query, limit)
        } else {
            dict::lookup_filtered(query, &assist_prefix, mode, limit)
        };

        // 错音容错：用户按「这个字的默认读音」打多音字词时（`zhongqing` 想打「重庆」），
        // 词库里那条词条挂在 chóng 下，正常查询永远出不来，用户只会以为没这个词。
        // `hint` 表里存了这些词的默认读音码，命中就**追加到末尾**——不抢任何正常
        // 候选的位置，因为撞码的 142 个变体（`zhongqing` 本来就有「钟情」）里，
        // 排在前面的一定是用户真正想要的。
        //
        // 带辅助码时不追加：辅助码是按**首字读音**算的，错音词的首字读音本来就不同，
        // 硬塞进来等于把用户刚做的那道筛选当没发生。
        if assist_prefix.is_empty() {
            FeiheTextService::append_wrong_reading(&mut list, query, config::pinyin_mode());
        }

        self.candidates.borrow_mut().clone_from(&list);
        self.selected.set(0);
        self.page.set(0);
    }

    /// 组合串显示内容。
    ///
    /// 分词模式下把每一段已选的词拼起来，当前段用 `[]` 括住——这样用户能从
    /// 组合串直接看出光标在哪一段、前后段选了什么。
    fn composition_text(&self) -> String {
        if !self.segments.borrow().is_empty() {
            let segs = self.segments.borrow();
            let cur = self.seg_idx.get().min(segs.len() - 1);
            let mut s = String::new();
            for (i, seg) in segs.iter().enumerate() {
                // 组合串要显示「将来会写进文档的样子」，否则开了繁体之后
                // 预览是简体、上屏是繁体，看着像 bug
                let word = crate::zhconv::apply(&seg.word);
                if i == cur {
                    s.push('[');
                    s.push_str(&word);
                    s.push(']');
                } else {
                    s.push_str(&word);
                }
            }
            s.push_str(&self.no_popup_hint());
            return s;
        }
        let code = self.code.borrow();
        let assist = self.assist.borrow();
        let mut s = if assist.is_empty() {
            code.clone()
        } else {
            format!("{code} {assist}")
        };
        s.push_str(&self.no_popup_hint());
        s
    }

    /// 不弹窗模式下，把当前这一页的候选拼到组合串后面。
    ///
    /// 为什么必须这么做：候选窗不弹了，用户就只剩「空格上屏第一候选」这一条路。
    /// 双拼同码字多（`de` 有几十个），看不见候选等于盲打——那还不如不用。
    /// 组合串是**宿主自己画在文档里的**，全屏游戏里通常仍能显示，所以把候选塞
    /// 进去是这条路下唯一还能给用户看的通道。
    ///
    /// 两种情况返回空串：开关没开；或者宿主已经接管了候选绘制（真 UILess）——
    /// 后者宿主自己会画一份，再塞进组合串就成了两份。
    fn no_popup_hint(&self) -> String {
        if self.ui_less.get() || !config::no_popup() {
            return String::new();
        }
        let cands = self.candidates.borrow();
        FeiheTextService::format_candidates(&cands, self.page.get())
    }

    /// 把一页候选拼成 `  1.你好 2.拟好 3.你号` 这样的尾部提示。
    ///
    /// 抽成纯函数才能单测——`no_popup_hint` 要 `&self`，而 `self` 里有 COM 句柄，
    /// 测试进程里造不出来。
    fn format_candidates(cands: &[Cow<'static, str>], page: usize) -> String {
        if cands.is_empty() {
            return String::new();
        }
        let start = page * candidate::PAGE_SIZE;
        let mut s = String::from("  ");
        for (i, w) in cands
            .iter()
            .skip(start)
            .take(candidate::PAGE_SIZE)
            .enumerate()
        {
            if i > 0 {
                s.push(' ');
            }
            s.push_str(&format!("{}.{}", i + 1, crate::zhconv::apply(w)));
        }
        s
    }

        // 当前选中的词；没有候选时退化成原始双拼码（保证空格能原样上屏）
        fn current_word(&self) -> String {
            // 分词模式：返回当前段已选的词
            if !self.segments.borrow().is_empty() {
                let segs = self.segments.borrow();
                let i = self.seg_idx.get().min(segs.len() - 1);
                if !segs[i].word.is_empty() {
                    return segs[i].word.clone();
                }
            }
            let idx = self.page.get() * candidate::PAGE_SIZE + self.selected.get();
            self.candidates
                .borrow()
                .get(idx)
                .map(|s| s.to_string())
                .unwrap_or_else(|| FeiheTextService::query_code(&self.code.borrow()).into_owned())
        }

}

// 以下方法需要 `self` 的 COM 接口（用来创建 ITfEditSession / ITfCompositionSink），
// 只能定义在 #[implement] 生成的 FeiheTextService_Impl 上。
impl FeiheTextService_Impl {
    fn start_composition(&self) {
        let code = self.code.borrow().clone();
        // 兜底：组合串只允许当前模式的编码键。万一有控制字符漏进来（例如宿主直接调
        // OnKeyDown 送来了 Ctrl / 退格），宁可不组合，也绝不写进文档。
        if code.is_empty() || !FeiheTextService::code_allowed(&code) {
            self.clear_state();
            return;
        }
        self.pending_op.borrow_mut().replace(EditOp::Start);
        self.request_edit_session();
    }

    fn update_composition(&self) {
        let text = self.composition_text();
        self.pending_op.borrow_mut().replace(EditOp::Update(text));
        self.request_edit_session();
    }

    /// Shift 按住期间按下别的键 → 记成组合键，抬起时不切中英
    fn note_shift_combo(&self, vk: u8) {
        if !FeiheTextService::is_shift(vk) && FeiheTextService::shift_down() {
            self.shift_used.set(true);
        }
    }

    /// 切中英：改状态、把没上屏的组合串提交掉、重绘浮窗
    fn toggle_chinese(&self) {        let next = !insert::chinese();
        insert::set_chinese(next);
        if self.is_composing() {
            self.commit_current();
        }
        statusbar::repaint();
    }

    fn commit_current(&self) {
        self.commit_with("");
    }

    /// 直接上屏一段文本，**不经过候选列表**。
    ///
    /// 一简次选 `d;` 用它：要出什么由键位决定，跟候选里选中的那个没关系。
    /// 别拿 `commit_with` 顶替——那个是「当前候选 + 后缀」，会拼出「的打」。
    fn commit_literal(&self, text: &str) {
        crate::freq::bump(&self.code.borrow(), text);
        self.pending_op
            .borrow_mut()
            .replace(EditOp::Commit(text.to_string()));
        self.request_edit_session();
        self.clear_state();
    }

    /// 提交当前候选，并在后面粘上一段文本（通常是全角标点）。
    ///
    /// 必须合成**一个** `EditOp`：`pending_op` 只有一个槽位，连着发 `Commit` 和
    /// `Insert` 会把前一个冲掉，结果是字没上屏、只剩标点。
    fn commit_with(&self, suffix: &str) {
        let word = self.current_word();
        // 学的是「拼上标点之前」的那个词。晚一步就会把「你好，」当成一个词记进去，
        // 而候选里永远不会有带标点的词，这条记录等于白存。
        // 码一起带上：同一个词在不同码下的次数要分开算（多音字）。
        crate::freq::bump(&self.code.borrow(), &word);
        let mut text = word;
        text.push_str(suffix);
        self.pending_op.borrow_mut().replace(EditOp::Commit(text));
        self.request_edit_session();
        self.clear_state();
    }

    /// 回车：把已经敲进去的编码原样上屏。
    ///
    /// 和空格的区别就在这儿——空格是「选第一个候选」，回车是「我就要这几个字母」。
    /// 快捷输入（`/rq`）例外：`/rq` 本身没有意义，退回第一个候选（当前日期）。
    /// 但**只敲了引导符本身**时又回到「原样上屏」：`/` 现在是引导符，不给这条
    /// 出口的话中文模式下就再也打不出斜杠了。
    fn commit_raw(&self) {
        let code = self.code.borrow().clone();
        let text = if code.starts_with(quick::PREFIX) {
            if code.chars().count() == 1 {
                FeiheTextService::prefix_literal(config::get().fullwidth).to_string()
            } else {
                self.current_word()
            }
        } else {
            // 手动分隔符是打字辅助，不该进文档：`ni'hao` 上屏的是 `nihao`
            FeiheTextService::query_code(&code).into_owned()
        };
        self.pending_op.borrow_mut().replace(EditOp::Commit(text));
        self.request_edit_session();
        self.clear_state();
    }

    fn cancel(&self) {
        self.pending_op.borrow_mut().replace(EditOp::Cancel);
        self.request_edit_session();
        self.clear_state();
    }

    /// 不建组合串，直接在光标处插入文本
    fn insert_direct(&self, text: &str) {
        self.pending_op
            .borrow_mut()
            .replace(EditOp::Insert(text.to_string()));
        self.request_edit_session();
    }

    /// 音节已经打完、但整串一个候选都没有时，切成多段交给用户逐段选。
    ///
    /// **这里绝不上屏。** 早期版本在这条路上加了「拆分上屏」：把左边能匹配的
    /// 部分直接提交、剩下的编码开新一轮。那是替用户做决定——他敲 `viuiq`
    /// 想打「指示器」，屏幕上却凭空出现「只是」，后面的 `qi` 又变成一个新的
    /// 单字组合。用户的原话：「我没有敲空格就不应该上屏任何字或者词」。
    ///
    /// 现在只有显式动作（空格 / 数字 / 回车 / 标点）才会把字写进文档。
    ///
    /// 触发条件：音节完整（双拼偶数长度 / 全拼能切出合法音节）、至少 2 个音节、
    /// 整串候选为空。切不出多段就什么都不做，等用户继续敲或退格。
    fn maybe_split(&self) {
        self.try_segment(&[], false);
    }

    /// 尝试进入分词模式。成功返回 `true`（已 refresh + 更新组合串）。
    ///
    /// `prev` 是上一轮的分段选择：继续敲字重新分段时，如果某一段的码没变，
    /// 就沿用用户已经选过的词。否则「选完第一段再往下打」会把选择冲掉。
    ///
    /// `force` 只由左右键传 `true`：**整串已经有候选也要进**。为什么必须留这个口子——
    /// 自造词是「整串一个码」，`limkyr` 学会「李明远」之后这个码的候选永远非空，
    /// 老条件就再也进不了分词模式，用户没法在同一个码下再造一个「李明源」。
    /// 自动触发（`maybe_split`）仍然要用非强制版：有候选就该正常选词。
    fn try_segment(&self, prev: &[(String, String, bool)], force: bool) -> bool {
        if self.assist_active.get() {
            return false;
        }
        let code = self.code.borrow().clone();
        if code.starts_with(quick::PREFIX) {
            return false; // 快捷输入不拆
        }
        if !force && !self.candidates.borrow().is_empty() {
            return false; // 有候选，正常走
        }
        let has = |p: &str| !FeiheTextService::lookup_any(p, 1).is_empty();
        let codes = FeiheTextService::plan_segments(&code, has);
        if codes.len() < 2 {
            return false;
        }

        let segments: Vec<Segment> = codes
            .iter()
            .enumerate()
            .map(|(i, c)| {
                // 码没变的段沿用上一轮的选择，连同「用户确认过」这个状态一起带过来
                let (word, picked) = match prev.get(i) {
                    Some((pc, w, pk)) if pc == c => (w.clone(), *pk),
                    _ => (
                        FeiheTextService::lookup_any(c, 1)
                            .first()
                            .map(|s| s.to_string())
                            .unwrap_or_default(),
                        false,
                    ),
                };
                Segment {
                    code: c.clone(),
                    word,
                    picked,
                }
            })
            .collect();

        let n = segments.len();
        *self.segments.borrow_mut() = segments;
        // 光标留在原来的段上（首次进入是 0），越界就夹到末段
        self.seg_idx.set(self.seg_idx.get().min(n - 1));
        self.refresh();
        self.update_composition();
        true
    }

    /// 退格：整串**只有一个候选**时，不删字母，改成切成段逐字选。
    ///
    /// 这是用户按搜狗手感要求的入口：打 `zuci` 出来「组词」，想再造一个「祖祠」，
    /// 以前只能用 `←` 硬拐进分词模式，退格却是「删一个字母」——一个键干两件事才对。
    ///
    /// 为什么要求「候选恰好一个」而不是「能拆就拆」：退格最常见的用法仍然是改错，
    /// 满屏候选时把它劫持成「逐字选」，用户改个错要按三次退格。只有整串已经**唯一**
    /// 命中一个词（`zuci`→组词）时，「这个码被一个词占满了」才成立，逐字选才有意义。
    fn backspace_into_segments(&self) -> bool {
        if !self.segments.borrow().is_empty() {
            return false;
        }
        if self.candidates.borrow().len() != 1 {
            return false;
        }
        if !self.try_segment(&[], true) {
            return false;
        }
        // 从第一个字开始，按顺序往右选
        self.seg_idx.set(0);
        self.refresh();
        self.update_composition();
        true
    }

    /// `←` / `→`：能拆就进分词模式并在段间移动，拆不了才退化成页内移动。
    ///
    /// 这是「用左右键一个字一个字选」的入口，也是**同码第二个自造词唯一能做出来的
    /// 路径**：`limkyr` 已经学会「李明远」之后整串就有候选，`try_segment` 的老条件
    /// （候选为空才拆）永远不成立，用户只能删掉编码重来。所以这里传 `force`。
    ///
    /// 单音节的码（`de`、`ni`）拆不开，原样保留页内移动的老手感：这类码本来
    /// 就没有「哪个字」可选，只有「哪个候选」可选。
    fn arrow_into_segments(&self, delta: isize) -> bool {
        if !self.try_segment(&[], true) {
            return false;
        }
        let n = self.segments.borrow().len() as isize;
        let i = (self.seg_idx.get() as isize + delta).clamp(0, n - 1);
        self.seg_idx.set(i as usize);
        self.refresh();
        self.update_composition();
        true
    }

    /// 分词模式下确认当前段的第 `sel` 个候选，光标后移一段；已经是最后一段就整体上屏。
    ///
    /// `sel` 是**全表下标**（`页号 * PAGE_SIZE + 页内序号`），和 `pick` 一致。
    /// 这里曾经收页内序号，于是分词态翻页后按数字键，命中的永远是第一页那个字：
    /// 用户打 `fwhe` 拆成 `[飞][鹤]`，翻到第二页点「鹤」，上屏的却是第一页的「合」。
    /// 单字（不分词）走 `pick`，那条路径一直是对的，所以 bug 只在组词时出现。
    fn confirm_segment(&self, sel: usize) {
        let n = self.segments.borrow().len();
        if n == 0 {
            return;
        }
        let idx = self.seg_idx.get().min(n - 1);
        let word = {
            let segs = self.segments.borrow();
            let list = FeiheTextService::lookup_any(
                &segs[idx].code,
                candidate_limit(&segs[idx].code),
            );
            list.get(sel)
                .or_else(|| list.first())
                .map(|s| s.to_string())
        };
        let Some(word) = word else { return };
        {
            let mut segs = self.segments.borrow_mut();
            segs[idx].word = word;
            // 用户按了空格/数字，这一段就是他明确要的字
            segs[idx].picked = true;
        }

        if idx + 1 < n {
            self.seg_idx.set(idx + 1);
            self.refresh();
            self.update_composition();
        } else {
            self.commit_segments("");
        }
    }

    /// 把「用户逐段拼出来的词」记成自造词（`phrase` 模块）。
    ///
    /// 场景：词库里没有的词（人名、公司名），用户把一长串编码切开、一段一段选字
    /// 拼出来。这个词只活到这次上屏为止——下一次敲同样的码，词库还是不认识它，
    /// 用户看到的就是「我刚组出来的词怎么又没了」。
    ///
    /// 下面四个判断都是为了**不记垃圾**，缺一个就会开始往表里灌废话：
    /// 1. 至少两段——单字不叫「组词」，单字的排序已经由动态词频管了
    /// 2. 每段都真有词——空段是 `merge_segments` 兜底时塞的段码，不是用户选的
    /// 3. 段码拼起来等于用户敲的那串——中途改过编码的话，码和词已经对不上了
    /// 4. 词库里本来就有这个词就跳过——自动切分沿用默认候选时不该记重复数据
    ///
    /// 全拼从词库反查（词库存的就是全拼），**任何一段查不到就整体放弃**：
    /// 拿不到可靠的全拼，这个词在换方案、开全拼模式之后就再也打不出来，
    /// 记了反而更糟（用户以为记下了）。
    fn remember_phrase(&self) {
        if !config::get().learn {
            return;
        }
        let code = FeiheTextService::query_code(&self.code.borrow()).into_owned();
        let mode = config::pinyin_mode();
        let segs = self.segments.borrow();
        if let Some((word, py)) =
            FeiheTextService::compose_phrase(&segs, &code, |c, w| dict::pinyin_of(c, w, mode))
        {
            crate::phrase::learn(&word, &py);
        }
    }

    /// 回车 / 拼音串末尾跟标点：上屏字母而不是候选（见 `raw_text`）。
    fn commit_raw_segments(&self, suffix: &str) {
        let mut text = {
            let segs = self.segments.borrow();
            FeiheTextService::raw_text(&segs)
        };
        // 确认过的段是用户明确选的词，词频照记；字母部分没有词可记，
        // 也不该进自造词表——那是一串英文，不是词。
        {
            let segs = self.segments.borrow();
            for s in segs.iter().filter(|s| s.picked && !s.word.is_empty()) {
                crate::freq::bump(&s.code, &s.word);
            }
        }
        text.push_str(suffix);
        self.segments.borrow_mut().clear();
        self.seg_idx.set(0);
        if text.is_empty() {
            self.cancel();
            return;
        }
        self.pending_op.borrow_mut().replace(EditOp::Commit(text));
        self.request_edit_session();
        self.clear_state();
    }

    /// 分词模式整体上屏：把所有段已选的词拼起来提交。
    ///
    /// 空段兜底交给 `merge_segments`。`suffix` 用来粘全角标点——必须和正文合成
    /// 一个 `EditOp`，否则标点会把正文那个待处理的 op 冲掉。
    fn commit_segments(&self, suffix: &str) {
        // 逐段学：分词上屏是「用户一段一段选出来的」，每一段都是明确的选择。
        // 必须在 merge 之前学——merge 出来的是整串（「指示器」），
        // 而候选是按段（「指示」+「器」）出来的，学整串没有下次能被命中的机会。
        {
            let segs = self.segments.borrow();
            for s in segs.iter() {
                crate::freq::bump(&s.code, &s.word);
            }
        }
        // 整串记成自造词：下次同样的码打满就直接出这个词，不用再拆一遍。
        // 放在逐段 bump 之后：`remember_phrase` 要读 segments，不能和上面的
        // 借用嵌套（它会再借一次 RefCell，同线程重入会 panic）。
        self.remember_phrase();
        let mut text = FeiheTextService::merge_segments(&self.segments.borrow());
        text.push_str(suffix);
        self.segments.borrow_mut().clear();
        self.seg_idx.set(0);
        if text.is_empty() {
            self.cancel();
            return;
        }
        self.pending_op.borrow_mut().replace(EditOp::Commit(text));
        self.request_edit_session();
        self.clear_state();
    }

    /// 退出分词模式，并退掉编码末尾的一个音节。
    fn exit_segment_mode(&self) {
        self.segments.borrow_mut().clear();
        self.seg_idx.set(0);
        self.pop_syllable();
    }

    /// 删掉编码末尾的一个完整音节，然后重新查候选。
    fn pop_syllable(&self) {
        let empty = {
            let mut code = self.code.borrow_mut();
            let next = FeiheTextService::drop_last_syllable(&code);
            *code = next;
            code.is_empty()
        };
        if empty {
            self.cancel();
        } else {
            self.refresh();
            self.update_composition();
        }
    }

    /// 分词模式下的退格：先「拆词」，拆不动再退一个音节。
    ///
    /// 搜狗那套手感的核心就是这一层：自动组出来的词不是你要的，按退格把它拆回
    /// 单个字，候选窗立刻变成这一段第一个字的候选，选完自动跳到下一个字——
    /// 于是「词库里没有的词」也能一个字一个字拼出来。
    ///
    /// 退格永远不丢编码（除非整串退空）：`limkyr` 退一次变 `[李][明]远`，
    /// 再退才删掉 `yr` 变 `limk`（李明）。
    fn segment_backspace(&self) {
        let idx = {
            let n = self.segments.borrow().len();
            if n == 0 {
                return;
            }
            self.seg_idx.get().min(n - 1)
        };
        let target = FeiheTextService::pick_expand_target(&self.segments.borrow(), idx);
        if let Some(t) = target {
            FeiheTextService::expand_segment(&mut self.segments.borrow_mut(), t);
            // 光标停在拆出来的第一段，从这个字开始往右重选
            self.seg_idx.set(t);
            self.refresh();
            self.update_composition();
            return;
        }
        // 全是单字了，再退就是删掉最后一个音节，回到「整串有候选」的普通态
        self.exit_segment_mode();
    }

    /// 分词模式的按键处理。`←`/`→` 移段，`↑`/`↓` 和 `-`/`=` 翻页，数字选词，
    /// 空格确认，回车上屏，退格拆词（拆不动再退音节），编码键延长编码并重新分段，
    /// 标点等其他键整体上屏后原样交给应用。
    fn on_segment_key(&self, vk: u8) -> Result<BOOL> {
        let n = self.segments.borrow().len();
        // 翻页：分词模式下候选是「当前这一段」的，翻的就是它的页
        if self.handle_page_key(vk) {
            return Ok(BOOL::from(true));
        }
        match vk {
            // 左：光标移到上一段
            0x25 => {
                let i = self.seg_idx.get();
                if i > 0 {
                    self.seg_idx.set(i - 1);
                    self.refresh();
                    self.update_composition();
                }
                Ok(BOOL::from(true))
            }
            // 右：光标移到下一段
            0x27 => {
                let i = self.seg_idx.get();
                if i + 1 < n {
                    self.seg_idx.set(i + 1);
                    self.refresh();
                    self.update_composition();
                }
                Ok(BOOL::from(true))
            }
            // 空格：确认当前段**高亮**的那个候选（和普通态 `commit_current` 同义）
            0x20 => {
                self.confirm_segment(self.page.get() * candidate::PAGE_SIZE + self.selected.get());
                Ok(BOOL::from(true))
            }
            // 回车：原样上屏字母，和普通组合态一个语义
            0x0D => {
                self.commit_raw_segments("");
                Ok(BOOL::from(true))
            }
            // 1-9：选当前段第 n 个候选（翻过页就要带上页偏移）
            0x31..=0x39 => {
                self.confirm_segment(
                    self.page.get() * candidate::PAGE_SIZE + (vk - 0x30) as usize - 1,
                );
                Ok(BOOL::from(true))
            }
            // 退格：先把当前段拆成单字（组词），拆不动再退一个音节
            0x08 => {
                self.segment_backspace();
                Ok(BOOL::from(true))
            }
            0x1B => {
                self.cancel();
                Ok(BOOL::from(true))
            }
            // 编码键：接着往下打就**延长编码、重新分段**，不能整体上屏。
            //
            // 这是长句输入的关键：用户打「指示器」+「在哪」会一路敲 `viuiqizdnc`，
            // 中途进来分词模式后如果按老逻辑「先整体上屏再把键交给应用」，
            // 屏幕上就会凭空出现半句话，后面那个字母还会被当成普通字符插进去。
            v if FeiheTextService::is_code_key(v) => {
                let Some(ch) = FeiheTextService::code_char(v) else {
                    return Ok(BOOL::from(true));
                };
                if ch == FeiheTextService::SEP
                    && !FeiheTextService::can_insert_sep(&self.code.borrow())
                {
                    return Ok(BOOL::from(true));
                }
                // 记下已选的词，重新分段时码没变的段沿用
                let prev: Vec<(String, String, bool)> = self
                    .segments
                    .borrow()
                    .iter()
                    .map(|s| (s.code.clone(), s.word.clone(), s.picked))
                    .collect();

                self.code.borrow_mut().push(ch);
                self.segments.borrow_mut().clear();
                self.refresh();
                // 新编码可能整串就有候选了（或音节还没打完），那就退出分词模式
                if !self.try_segment(&prev, false) {
                    self.update_composition();
                }
                Ok(BOOL::from(true))
            }
            // 其他键（标点、Tab 之类）：整体上屏，键再交给应用。
            // 标点上屏是主流输入法的通行做法，也是长句的自然结束方式。
            // 全角标点自己粘上去并吞键，理由同普通组合态。
            _ => {
                if let Some(ch) = FeiheTextService::fullwidth_punct_for(vk) {
                    let mut buf = [0u8; 4];
                    let punct = ch.encode_utf8(&mut buf);
                    // 有段根本选不出字（音节不像拼音，多半是在打英文）→ 整体上屏
                    // 会拼出「面ap，」这种东西，按字母上屏才对。
                    let has_letters = self.segments.borrow().iter().any(|s| s.word.is_empty());
                    if has_letters {
                        self.commit_raw_segments(punct);
                    } else {
                        self.commit_segments(punct);
                    }
                    return Ok(BOOL::from(true));
                }
                self.commit_segments("");
                Ok(BOOL::from(false))
            }
        }
    }

    fn backspace(&self) {
        // 辅助码态：先删辅助码，删空后退回双拼码
        if self.assist_active.get() {
            let empty = {
                let mut a = self.assist.borrow_mut();
                a.pop();
                a.is_empty()
            };
            if empty {
                self.assist_active.set(false);
                // 用户把这键退了，说明他不是要辅助码（`ddj` 想打「代价」）。
                // 记下来，等他重新敲 `j` 时别再自动拆一次，否则按几次退格都白搭。
                self.assist_direct_denied.set(true);
            }
            self.refresh();
            self.update_composition();
            return;
        }
        // 整串唯一命中一个词：退格不删字母，改成逐字选（再造一个同码词）
        if self.backspace_into_segments() {
            return;
        }
        let empty = {
            let mut c = self.code.borrow_mut();
            c.pop();
            c.is_empty()
        };
        if empty {
            self.cancel();
        } else {
            self.refresh();
            self.update_composition();
        }
    }

    /// 数字键选词（n 从 1 开始）
    fn pick(&self, n: usize) {
        let idx = self.page.get() * candidate::PAGE_SIZE + (n - 1);
        let word = self.candidates.borrow().get(idx).map(|s| s.to_string());
        match word {
            Some(w) => {
                self.pending_op.borrow_mut().replace(EditOp::Commit(w));
                self.request_edit_session();
                self.clear_state();
            }
            None => self.commit_current(),
        }
    }

    /// `Ctrl+数字`：把这个候选**钉**到当前编码上，然后照常上屏。
    ///
    /// 返回是否真的做了。做不到（没有这个候选、快捷输入态）就返回 false，
    /// 让按键落回「修饰键组合 → 提交组合串并交还应用」的老路。
    ///
    /// 为什么不另开一个「置顶模式」：置顶是「我看到这个候选不对，换一个」的
    /// 即时动作，最自然的入口就是选词本身——`Ctrl` 表示「这次选词顺便记住」。
    /// 取出「第 idx 个候选」以及它该钉在哪个码上。
    ///
    /// idx 是**全表下标**（`page * PAGE_SIZE + 页内序号`）。快捷输入的候选是
    /// 「2026年10月23日」这种生成出来的串，钉它没有意义，直接返回 None。
    fn pin_target(&self, idx: usize) -> Option<(String, String)> {
        if self.code.borrow().starts_with(quick::PREFIX) {
            return None;
        }
        let word = self.candidates.borrow().get(idx)?.to_string();
        // 分词模式下候选属于**当前这一段**，置顶也要钉到那一段的码上，
        // 钉整串会让下次打同一个词时对不上。
        let code = if self.segments.borrow().is_empty() {
            FeiheTextService::query_code(&self.code.borrow()).into_owned()
        } else {
            let segs = self.segments.borrow();
            let i = self.seg_idx.get().min(segs.len() - 1);
            segs[i].code.clone()
        };
        if code.is_empty() {
            return None;
        }
        Some((code, word))
    }

    fn pin_candidate(&self, n: usize) -> bool {
        let idx = self.page.get() * candidate::PAGE_SIZE + (n - 1);
        let Some((code, word)) = self.pin_target(idx) else {
            return false;
        };
        crate::pin::toggle(&code, &word);

        self.pending_op.borrow_mut().replace(EditOp::Commit(word));
        self.request_edit_session();
        self.clear_state();
        true
    }

    /// 右键：切换置顶，**不提交**。
    ///
    /// 和 `Ctrl`+数字 的区别就在这里——后者顺带把词上屏（置顶是它的副作用），
    /// 而多页签程序里 `Ctrl+1..9` 被浏览器抢去切页签，用户需要一个纯鼠标入口。
    /// 切换后立刻重新查候选：钉上的词跳到第一位、取消的回到原位，这就是反馈，
    /// 不用再弹任何提示。
    fn toggle_pin_candidate(&self, page_index: usize) -> bool {
        let idx = self.page.get() * candidate::PAGE_SIZE + page_index;
        let Some((code, word)) = self.pin_target(idx) else {
            return false;
        };
        crate::pin::toggle(&code, &word);
        self.refresh();
        self.show_candidates();
        true
    }

    /// 把候选窗的点击路由回本对象。
    ///
    /// 存裸地址是必要的：windows-rs 的 `#[implement]` 只生成「结构体 → 接口」，
    /// 没有「接口 → 结构体」的路径，而点击回调需要的正是 `_Impl` 上的
    /// `pick` / `confirm_segment`——它们要 `to_interface()` 去开 edit session，
    /// 光有内部数据结构的引用办不到。
    ///
    /// 安全性靠 `alive` 令牌：闭包只持有 `Weak`，对象析构后 `upgrade()` 失败，
    /// 就不会去解引用一个已经释放的地址。地址本身在 COM 对象生命周期内不变
    /// （宏把对象分配在堆上，之后不会移动）。
    fn install_click_handler(&self) {
        let addr = self as *const FeiheTextService_Impl as usize;
        let alive = Arc::downgrade(&self.alive);
        let alive2 = alive.clone();
        crate::candidate::set_click_handler(addr, move || {
            let alive = alive.clone();
            Arc::new(move |index: usize| {
                let Some(_token) = alive.upgrade() else {
                    return;
                };
                // SAFETY: 令牌还活着 ⇒ 对象没析构；地址在生命周期内不变。
                let this = unsafe { &*(addr as *const FeiheTextService_Impl) };
                this.on_candidate_click(index);
            })
        });
        crate::candidate::set_right_click_handler(addr, move || {
            Arc::new(move |index: usize| {
                let Some(_token) = alive2.upgrade() else {
                    return;
                };
                // SAFETY: 同上——令牌活着说明对象没析构，地址在生命周期内不变
                let this = unsafe { &*(addr as *const FeiheTextService_Impl) };
                this.on_candidate_right_click(index);
            })
        });
    }

    /// 右键点候选：切换置顶（不上屏、不打断组合态）。
    fn on_candidate_right_click(&self, index: usize) {
        if !self.is_composing() {
            return;
        }
        self.toggle_pin_candidate(index);
    }

    /// 候选窗被点了页内第 `index` 项（0 基）。
    ///
    /// 行为和对应的数字键完全一致——鼠标只是「按数字键」的另一种入口，
    /// 不应该有第二套语义。
    fn on_candidate_click(&self, index: usize) {
        // 窗口可能还挂在屏幕上，但组合态已经结束（宿主刚提交、或按了 Esc）
        if !self.is_composing() {
            return;
        }
        if !self.segments.borrow().is_empty() {
            // 分词模式：确认当前段并跳到下一段（最后一段则整体上屏）。
            // `index` 是页内序号，`confirm_segment` 收全表下标——翻过页就必须补上页偏移。
            self.confirm_segment(self.page.get() * candidate::PAGE_SIZE + index);
        } else {
            // 普通模式的 `pick` 是 1 基（数字键 1-9），这里补 1
            self.pick(index + 1);
        }
    }
}

impl FeiheTextService {
    fn move_selection(&self, delta: isize) {
        let total = self.candidates.borrow().len();
        if total == 0 {
            return;
        }
        let start = self.page.get() * candidate::PAGE_SIZE;
        let count = (total - start).min(candidate::PAGE_SIZE) as isize;
        let next = (self.selected.get() as isize + delta).clamp(0, count - 1);
        self.selected.set(next as usize);
        self.show_candidates();
    }

    /// 翻页键：`-` / `↑` 上一页，`=` / `↓` 下一页。返回方向，不是翻页键返回 None。
    ///
    /// `-` `=` 是搜狗、微软拼音、QQ 输入法的通行键位；`↑` `↓` 保留给习惯方向键的人。
    /// **只在组合态生效**——非组合态 `-` `=` 还是全角标点 `－` `＝`（见 `fullwidth_punct`）。
    fn page_delta(vk: u8) -> Option<isize> {
        match vk {
            0x26 => Some(-1), // ↑
            0x28 => Some(1),  // ↓
            v if v == VK_OEM_MINUS.0 as u8 => Some(-1),
            v if v == VK_OEM_PLUS.0 as u8 => Some(1),
            _ => None,
        }
    }

    /// 翻页键处理：是翻页键就翻页并返回 true（键已吃掉）。
    ///
    /// 哪怕只有一页也吃掉，行为才可预测——否则「按了没反应」和「突然上屏一个＝」
    /// 会随机出现。想打 `－` `＝` 先按空格把候选上屏再敲。
    fn handle_page_key(&self, vk: u8) -> bool {
        match Self::page_delta(vk) {
            Some(d) => {
                self.turn_page(d);
                true
            }
            None => false,
        }
    }

    fn turn_page(&self, delta: isize) {
        let total = self.candidates.borrow().len();
        if total == 0 {
            return;
        }
        let pages = total.div_ceil(candidate::PAGE_SIZE) as isize;
        let next = (self.page.get() as isize + delta).clamp(0, pages - 1);
        if next as usize != self.page.get() {
            self.page.set(next as usize);
            self.selected.set(0);
            self.show_candidates();
        }
    }

    fn show_candidates(&self) {
        // 先克隆一份快照，避免借用跨过 candidate::show（它会加锁、发窗口消息）
        let snapshot: Vec<Cow<'static, str>> = self.candidates.borrow().clone();
        if snapshot.is_empty() {
            self.hide_candidates();
            return;
        }

        // 宿主声明了 UI-less（独占全屏游戏）：先问它接不接。
        // 接了它自己画，我们一个窗口都不弹——这是「全屏游戏里看得见候选」的通道。
        if self.ui_less.get() {
            let items: Vec<String> = snapshot.iter().map(|s| s.to_string()).collect();
            let (sel, page) = (self.selected.get(), self.page.get());
            let sess = self.uielement.borrow().as_ref().cloned();
            let handled = match sess {
                Some(s) => s.update(items, page * candidate::PAGE_SIZE + sel, page),
                None => false,
            };
            if handled {
                return;
            }
            // 宿主不接，或者拿不到 ITfUIElementMgr：降级回自绘窗，
            // 本轮激活里不再问它（每次按键都问一遍是纯浪费）。
            self.ui_less.set(false);
        }

        let items: Vec<&str> = snapshot.iter().map(|s| s.as_ref()).collect();
        candidate::show(
            &items,
            self.selected.get(),
            self.page.get(),
            self.caret_x.get(),
            self.caret_y.get(),
        );
    }

    /// 记住光标屏幕位置并刷新候选窗
    fn remember_caret(&self, context: &ITfContext, ec: u32, range: &ITfRange) {
        if let Some(rc) = unsafe { caret_screen_rect(context, ec, range) } {
            self.caret_x.set(rc.left);
            self.caret_y.set(rc.bottom);
            // 状态浮窗的「跟随光标」模式靠这个位置
            statusbar::set_caret(rc.left, rc.bottom);
        } else {
        }
        self.show_candidates();
    }

}

impl FeiheTextService_Impl {
    /// 在光标处开一个新的组合串，文本取 `composition_text()`。
    ///
    /// 只在 `EditOp::Start` 里用。注意组合串一旦 `EndComposition`，文档里那串字
    /// 就落定了——所以任何「继续往下打」的路径都必须走 `EditOp::Update`
    /// 改现有组合，绝不能靠「结束旧的 + 开新的」来模拟。
    unsafe fn begin_composition(&self, context: &ITfContext, ec: u32) -> Result<()> {
        let text = self.composition_text();
        let Ok(range) = caret_range(context, ec) else {
            return Err(Error::new(E_FAIL, "no caret range"));
        };
        let comp_mgr: ITfContextComposition = context.cast()?;
        let sink: ITfCompositionSink = self.to_interface();
        let comp = comp_mgr.StartComposition(ec, &range, &sink)?;

        let r = comp.GetRange()?;
        r.SetText(ec, 0, &to_utf16(&text))?;
        r.Collapse(ec, TF_ANCHOR_END)?;
        put_caret(context, ec, &r);

        self.composition.borrow_mut().replace(comp);
        self.remember_caret(context, ec, &r);
        Ok(())
    }

    fn request_edit_session(&self) {
        let Some(context) = self.context.borrow().as_ref().cloned() else {
            return;
        };
        let session: ITfEditSession = self.to_interface();
        let hr = unsafe {
            context.RequestEditSession(self.tid.get(), &session, TF_ES_SYNC | TF_ES_READWRITE)
        };
        if hr.is_err() {
            self.pending_op.borrow_mut().take();
        }
    }
}

impl Drop for FeiheTextService {
    fn drop(&mut self) {
        // 兜底：对象销毁时反注册 key sink，避免 MSCTF 持有悬空指针
        self.deactivate_sink();
        dllcount::object_destroyed();
    }
}

// ─────────────────────────────────────────────────────────────
// 生命周期
// ─────────────────────────────────────────────────────────────

impl ITfTextInputProcessor_Impl for FeiheTextService_Impl {
    fn Activate(&self, ptim: Ref<'_, ITfThreadMgr>, tid: u32) -> Result<()> {
        let tim: ITfThreadMgr = ptim.ok()?.clone();

        // 焦点文档不一定已存在，容错处理（否则 Activate 失败会让服务被判为加载失败）
        let context = unsafe { tim.GetFocus() }
            .ok()
            .and_then(|doc| unsafe { doc.GetTop() }.ok());

        // 读配置并按当前双拼方案准备词库索引（方案变了会重建，约 30ms）
        let cfg = config::load();
        dict::set_scheme(cfg.scheme);

        // 别的进程可能刚重编译了 user.bin（设置窗里点「编译成二进制」、或导入词库）。
        // 本进程还映射着旧文件，看不到新词；每次激活探一次（两次文件属性查询，可忽略）。
        dict::refresh_user_bin();

        // 同理，别的宿主进程可能刚学过词。不拉过来的话，在 Word 里把「诊断」
        // 选到第一位，切到浏览器里它又沉回去了——用户会觉得这功能时灵时不灵。
        // `refresh` 内部按 mtime 判断，没变化就只做一次 stat。
        crate::freq::refresh();

        // 置顶表同理：在设置窗里清空、或手改了 pinned.txt，别的进程要能看见。
        crate::pin::refresh();

        // 自造词同理：在别的宿主里刚拼出来的词，切过来也要能打。同样按 mtime 判断。
        crate::phrase::refresh();

        // 自动检测「是不是全屏」的结果也丢掉重问：用户刚从全屏游戏 Alt+Tab 回来，
        // 不该还要等缓存过期才看得见候选窗。
        crate::fullscreen::invalidate();

        // 候选窗鼠标点击：把「点了第几个」路由回本对象。
        // 放在 Activate 里而不是每次 show——闭包要捕获对象地址，只在对象身份
        // 变化时才需要重建，`set_click_handler` 内部按 id 判断。
        self.install_click_handler();

        // 中英文开关。关着的时候 MSCTF 不会把按键交给 TIP，症状就是
        // 「切到飞鹤拼音后字母直接进应用、没有任何候选窗」。
        //
        // 只有在「我们认为当前是中文态」时才强制置 1：用户主动切到英文后
        // 不能再把它拉回来，否则他刚做的切换会被下一次激活抹掉。
        if insert::chinese() {
            open_keyboard(&tim, tid);
        } else {
        }

        let keystroke_mgr: ITfKeystrokeMgr = tim.cast()?;
        let key_sink: ITfKeyEventSink = self.to_interface();
        // 第三个参数 fForeground 必须是 TRUE。
        // 官方文档：TRUE 才把本 sink 设为「前台 key sink」，而
        // 「前台 key sink 接收所有键盘事件；非前台 key sink 只接收 preserved key
        // 和发生在自己拥有的文本上的按键」。
        // 传 FALSE 会导致 Activate 成功、sink 挂上，但 OnTestKeyDown 永远不被调用，
        // 表现就是「切到输入法后字母直接进应用、没有候选窗」。
        let hr = unsafe { keystroke_mgr.AdviseKeyEventSink(tid, &key_sink, true) };
        hr?;

        self.thread_mgr.borrow_mut().replace(tim);
        *self.context.borrow_mut() = context;
        self.tid.set(tid);
        // 软键盘 / 设置窗要靠这个 client id 才能 RequestEditSession 和改中英开关
        insert::set_client_tid(tid);
        statusbar::show();
        Ok(())
    }

    fn Deactivate(&self) -> Result<()> {
        self.deactivate_sink();
        // 浮窗、软键盘、设置窗都属于本 TIP，反激活时一并销毁，
        // 否则切走输入法后它们还挂在屏幕上（而且窗口过程指针会随 DLL 卸载失效）。
        statusbar::destroy();
        softkbd::destroy();
        settings::destroy();
        crate::font::destroy();
        Ok(())
    }
}

/// 判断宿主是不是要 UILess。两条路都要看，因为**参数不一定到得了**：
///
/// 1. 正常情况下 MSCTF 会把宿主的 `TF_TMAE_UIELEMENTENABLEDONLY`(4) 转成
///    `TF_AE_UIELEMENTENABLEDONLY`(1) 传进 `dwflags`；
/// 2. 但 TIP 常常不是由宿主的 `ActivateEx` 拉起来的——user32 的 IMM32 兼容层会在
///    建窗口 / 拿焦点时抢先把它激活一遍，那时 `dwflags` 恒为 0。宿主的意图这时只
///    留在线程管理器上，`GetActiveFlags` 里读得到（实测为 0x80000004）。
///
/// 只认第 1 条的话，真实游戏里永远进不了 UILess，而且一点错都不报。见
/// `tools/tsfhost` 里的实测记录。
fn ui_less_requested(tim: &ITfThreadMgr, dwflags: u32) -> bool {
    if dwflags & uielement::TF_AE_UIELEMENTENABLEDONLY != 0 {
        return true;
    }
    let Ok(ex) = tim.cast::<ITfThreadMgrEx>() else {
        return false;
    };
    unsafe { ex.GetActiveFlags() }
        .map(|f| f & uielement::TF_TMAE_UIELEMENTENABLEDONLY != 0)
        .unwrap_or(false)
}

impl ITfTextInputProcessorEx_Impl for FeiheTextService_Impl {
    fn ActivateEx(&self, ptim: Ref<'_, ITfThreadMgr>, tid: u32, dwflags: u32) -> Result<()> {
        // 先留一份再交给 Activate——`Ref` 不实现 Copy，先 clone 才不会被 move 走
        let tim: ITfThreadMgr = ptim.ok()?.clone();
        self.Activate(ptim, tid)?;

        // 宿主把线程管理器按 UI-less 方式激活了（典型：独占全屏游戏）。全屏下 DWM
        // 被绕开，我们自己开的窗口画了也不上屏，候选只能交给宿主画——见 `uielement`。
        let ui_less = ui_less_requested(&tim, dwflags);
        self.ui_less.set(ui_less);
        if ui_less {
            self.setup_uielement();
            // 会话没建起来（拿不到 ITfUIElementMgr）就不算 UILess，浮窗照旧
            if self.ui_less.get() {
                // 状态浮窗同理：全屏下它只会变成一个看不见的窗口
                statusbar::hide();
            }
        }
        Ok(())
    }
}

impl FeiheTextService_Impl {
    /// 建 UILess 会话。
    ///
    /// 拿不到 `ITfUIElementMgr`（很老的系统，或宿主不是 MSCTF 线程管理器）就退回
    /// 自绘窗——宁可候选窗看不见，也不能让按键流程因为少个接口就断掉。
    fn setup_uielement(&self) {
        let tim = self.thread_mgr.borrow().as_ref().cloned();
        let Some(tim) = tim else {
            self.ui_less.set(false);
            return;
        };
        let Ok(mgr) = tim.cast::<ITfUIElementMgr>() else {
            self.ui_less.set(false);
            return;
        };
        // 元素要报自己的 document manager，宿主可能用它判断候选属于哪个文档
        let doc = unsafe { tim.GetFocus() }.ok();
        let sess = uielement::Session::new(mgr, doc);
        self.uielement.borrow_mut().replace(Rc::new(sess));
    }
}

// ─────────────────────────────────────────────────────────────
// 按键
// ─────────────────────────────────────────────────────────────

impl ITfKeyEventSink_Impl for FeiheTextService_Impl {
    fn OnSetFocus(&self, _fforeground: BOOL) -> Result<()> {
        // 失焦即提交，避免组合串残留在别人的文档里
        if self.is_composing() {
            self.commit_current();
        }
        Ok(())
    }

    fn OnTestKeyDown(
        &self,
        _pic: Ref<'_, ITfContext>,
        wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        let vk = (wparam.0 & 0xff) as u8;
        // 记住「Shift 是按着不动的组合键」——经典宿主里 Shift 自己的 OnKeyDown
        // 收不到（OnTestKeyDown 拒了就不派发），所以只能在这里做标记。
        self.note_shift_combo(vk);
        if FeiheTextService::modifiers_down() {
            return Ok(BOOL::from(false));
        }
        // 英文态：一个键都不吃，全部交给应用。
        // 这一步不能省——宿主不会替我们挡住按键（见 insert::CHINESE 的注释）。
        if !insert::chinese() {
            return Ok(BOOL::from(false));
        }
        if self.is_composing() {
            return Ok(BOOL::from(true));
        }
        // 除了编码键，**全角标点键也必须认**。经典宿主（记事本、Word）里
        // OnTestKeyDown 返回 FALSE 就不会派发 OnKeyDown，我们连看都看不到这个键，
        // 全角标点开关等于失效。返回 TRUE 只是「让我看一眼」，OnKeyDown 里
        // 该拒绝的照样返回 FALSE，键最终还是会交给应用。
        let take = FeiheTextService::is_code_key(vk)
            || FeiheTextService::fullwidth_punct_for(vk).is_some();
        Ok(BOOL::from(take))
    }

    fn OnKeyDown(
        &self,
        pic: Ref<'_, ITfContext>,
        wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        let vk = (wparam.0 & 0xff) as u8;

        // 在应用里敲键就说明用户已经回到打字，软键盘可以收了
        softkbd::hide();

        if let Ok(context) = pic.ok() {
            self.context.borrow_mut().replace(context.clone());
        } else {
        }

        // ── Shift：单击切中英 ──────────────────────────────
        // 按下时什么也不做，键照常交给应用（大写字母要用它）；抬起时才判定。
        self.note_shift_combo(vk);
        if FeiheTextService::is_shift(vk) {
            return Ok(BOOL::from(false));
        }

        // ── Ctrl+Shift+U：临时切换「弹不弹窗」 ──────────────
        //
        // 打开后会连状态浮窗一起藏起来，而浮窗上的「设置」是进设置窗的**唯一**
        // 入口。不给个键盘出口的话，用户在「始终不弹」下就把自己锁在外面了，
        // 只能去手改 config.ini。所以这个组合键必须存在，而且必须在下面那条
        // 「修饰键一律交还应用」之前拦下来。
        //
        // 它切的是**运行时覆盖**（`config::toggle_popup_override`），不写配置：
        // 这是「这一会儿先这么着」的应急键，真想要长期行为去设置里改。
        // 也因此它按一次覆盖、再按一次回到「跟随设置（含自动检测）」。
        //
        // 键位沿用 U（这个开关以前叫 UILess，老用户已经记住了）；它和宿主驱动的
        // 真 UILess 不是一回事——那个没有开关，由 `ActivateEx` 的 dwflags 决定。
        //
        // 必须放在 `is_shift` 判断之后：Shift 按下时那条已经先返回了，
        // 而 `note_shift_combo` 也记下了「Shift 是配合别的键用的」，
        // 抬起时不会误切中英。
        if FeiheTextService::ctrl_only() && FeiheTextService::shift_down() && vk == 0x55 {
            config::toggle_popup_override();
            // 不管切到哪一档，都用「当前的实际结论」决定窗口该显示还是收起
            if config::no_popup() {
                self.hide_candidates();
                statusbar::hide();
            } else {
                statusbar::show();
            }
            if self.is_composing() {
                self.commit_current();
            }
            return Ok(BOOL::from(true));
        }

        // ── Shift + 字母：直接上屏大写 ──────────────────────
        //
        // 中文态下 Shift 不进编码，`A` 和 `Shift+A` 都会往组合串里塞一个 `a`，
        // 于是「按住 Shift 想打大写字母」和直接敲键没区别。用户按住 Shift 想要
        // 的显然是大写字母本身，所以这里把已有组合串先结掉，再把大写字母直插文档。
        //
        // 位置有讲究：必须在 `modifiers_down()` 之前（那里会把键交还应用，大写
        // 就变成「先切英文再打」），也必须在 `Ctrl+Shift+U` 之后（那个组合同样
        // 带 Shift，不能被这里抢走）。条件里排除 Ctrl/Alt/Win，`Ctrl+Shift+A`
        // 这类系统快捷键仍然原样交还。
        if !FeiheTextService::modifiers_down() && FeiheTextService::shift_down() {
            if let Some(ch) = FeiheTextService::shifted_letter(vk) {
                let mut buf = [0u8; 4];
                let letter = ch.encode_utf8(&mut buf);
                // 上屏和提交必须合成一个 EditOp（`pending_op` 只有一个槽位），
                // 两个函数都接受 suffix，正好一起粘上去。
                if !self.segments.borrow().is_empty() {
                    self.commit_segments(letter);
                } else if self.is_composing() {
                    self.commit_with(letter);
                } else {
                    self.insert_direct(letter);
                }
                return Ok(BOOL::from(true));
            }
        }

        // 组合态下按修饰键（Ctrl / Alt / Win）：先把组合串提交，键原样交给应用。
        // 否则 Ctrl+A、Ctrl+C 会被当成字母吃进组合串。
        //
        // 唯一的例外是 Ctrl+数字 = 「置顶当前候选」。它必须在下面那条之前处理，
        // 否则会被当成普通修饰键组合直接提交掉。组合态下按 Ctrl+1 没人是想切
        // 浏览器标签页——那会儿用户正盯着候选窗选词。
        if FeiheTextService::ctrl_only() && (0x31..=0x39).contains(&vk) && self.is_composing() {
            if self.pin_candidate((vk - 0x30) as usize) {
                return Ok(BOOL::from(true));
            }
        }

        if FeiheTextService::modifiers_down() {
            if self.is_composing() {
                self.commit_current();
            }
            return Ok(BOOL::from(false));
        }

        // 英文态：不组合、不拦键
        if !insert::chinese() {
            if self.is_composing() {
                self.commit_current();
            }
            return Ok(BOOL::from(false));
        }

        // 分词模式：左右移段、数字选词、空格确认、回车整体上屏
        if !self.segments.borrow().is_empty() {
            return self.on_segment_key(vk);
        }

        if !self.is_composing() {
            // 注意：WinUI3 / CoreText 宿主不会调用 OnTestKeyDown，OnKeyDown 会收到
            // 任意按键（实测 Ctrl、退格都会进来）。这里必须自己过滤，只认字母，
            // 否则控制字符会被塞进组合串并 SetText 到文档里，表现为「打出符号」。
            if FeiheTextService::is_code_key(vk) {
                let Some(ch) = FeiheTextService::code_char(vk) else {
                    return Ok(BOOL::from(false));
                };
                // 分隔符不能开新组合串（没有可分隔的东西），落到下面的全角标点分支
                if ch != FeiheTextService::SEP {
                    self.code.borrow_mut().push(ch);
                    self.refresh();
                    self.start_composition();
                    return Ok(BOOL::from(true));
                }
            }

            // 全角标点：自己插字，不建组合串
            if let Some(ch) = FeiheTextService::fullwidth_punct_for(vk) {
                let mut buf = [0u8; 4];
                self.insert_direct(ch.encode_utf8(&mut buf));
                return Ok(BOOL::from(true));
            }
            return Ok(BOOL::from(false));
        }

        // - / = 和 ↑ / ↓：翻页。放在 match 之前，因为 match 里没法写
        // `VK_OEM_MINUS.0 as u8` 这种带转换的模式；而且分词模式要走同一套，
        // 所以抽成 handle_page_key 而不是在 match 里列四条 arm。
        if self.handle_page_key(vk) {
            return Ok(BOOL::from(true));
        }

        match vk {
            // 空格：提交选中候选
            0x20 => {
                self.commit_current();
                Ok(BOOL::from(true))
            }
            // 回车：把已输入的编码原样上屏，不选候选
            0x0D => {
                self.commit_raw();
                Ok(BOOL::from(true))
            }
            // Esc：取消
            0x1B => {
                self.cancel();
                Ok(BOOL::from(true))
            }
            // 退格：删一个双拼键
            0x08 => {
                self.backspace();
                Ok(BOOL::from(true))
            }
            // 1-9：选第 n 个候选
            0x31..=0x39 => {
                self.pick((vk - 0x30) as usize);
                Ok(BOOL::from(true))
            }
            // 左右：先把整串切成段逐字选（同码第二个自造词就靠这条路），
            // 拆不开的（单个音节）才退回页内移动
            0x25 => {
                if !self.arrow_into_segments(-1) {
                    self.move_selection(-1);
                }
                Ok(BOOL::from(true))
            }
            0x27 => {
                if !self.arrow_into_segments(1) {
                    self.move_selection(1);
                }
                Ok(BOOL::from(true))
            }
            // 上下：翻页（和 - / = 一起由 handle_page_key 统一处理）
            // 这里不再单独列 0x26 / 0x28，避免两处逻辑走岔
            // Tab：进入/退出辅助码筛选（辅助码关闭时把 Tab 还给应用）
            0x09 => {
                let mode = config::get().assist_mode;
                if mode == AssistMode::Off {
                    self.commit_current();
                    return Ok(BOOL::from(false));
                }
                let next = !self.assist_active.get();
                self.assist_active.set(next);
                if next {
                    // 连打之后再按 Tab：末尾的形码该挪进辅助码（`ddj` → `dd j`）。
                    // 只动编码，不动候选——随后 `refresh` 统一重算。
                    if !config::pinyin_mode() {
                        let code = self.code.borrow().clone();
                        if let Some((head, tail)) =
                            FeiheTextService::split_trailing_assist(&code, mode.max_keys(), |p, a| {
                                FeiheTextService::assist_hit(p, a, mode)
                            })
                        {
                            self.code.borrow_mut().clone_from(&head);
                            self.assist.borrow_mut().clone_from(&tail);
                        }
                    }
                } else {
                    self.assist.borrow_mut().clear();
                    // Tab 退出 = 明确说「这键不是辅助码」，同一次输入里别再自动拆
                    self.assist_direct_denied.set(true);
                }
                self.refresh();
                self.update_composition();
                Ok(BOOL::from(true))
            }
            // 编码键：累积双拼码；辅助码态下累积辅助码
            v if FeiheTextService::is_code_key(v) => {
                if let Some(ch) = FeiheTextService::code_char(v) {
                    // 引导符只能出现在开头，否则吞掉不改码
                    if ch == quick::PREFIX && !self.code.borrow().is_empty() {
                        return Ok(BOOL::from(true));
                    }
                    if ch == FeiheTextService::SEP {
                        // 一简次选键：单键后面本来就插不了分隔符（见 `brief_alt`），
                        // 分号在这个位置是空键，接过来当 `d;` → 打。
                        if vk == VK_OEM_1.0 as u8 {
                            let alt = FeiheTextService::brief_alt(&self.code.borrow());
                            if let Some(w) = alt {
                                self.commit_literal(w);
                                return Ok(BOOL::from(true));
                            }
                        }
                        // 手动分隔：位置不合理就吞掉，不打扰用户
                        if !FeiheTextService::can_insert_sep(&self.code.borrow()) {
                            return Ok(BOOL::from(true));
                        }
                        self.code.borrow_mut().push(ch);
                    } else if self.assist_active.get() {
                        self.assist.borrow_mut().push(ch);
                    } else {
                        self.code.borrow_mut().push(ch);
                    }
                }
                // 3 键直出：默认关，开了就在 refresh 之前把末尾那 1 键挪进辅助码
                self.auto_assist();
                self.refresh();
                self.update_composition();
                self.maybe_split();
                Ok(BOOL::from(true))
            }
            // 其他键：先提交，再把键还给应用。
            //
            // 全角标点例外——不能把键还给应用，应用会插半角。自己粘上去，
            // 并且要吞掉这个键（返回 true），否则用户会得到「，,」两个字符。
            _ => {
                if let Some(ch) = FeiheTextService::fullwidth_punct_for(vk) {
                    let mut buf = [0u8; 4];
                    self.commit_with(ch.encode_utf8(&mut buf));
                    return Ok(BOOL::from(true));
                }
                self.commit_current();
                Ok(BOOL::from(false))
            }
        }
    }

    fn OnTestKeyUp(
        &self,
        _pic: Ref<'_, ITfContext>,
        wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        // 只有 Shift 需要「测试」，其它键抬起我们都不关心
        let vk = (wparam.0 & 0xff) as u8;
        Ok(BOOL::from(FeiheTextService::is_shift(vk)))
    }

    fn OnKeyUp(
        &self,
        _pic: Ref<'_, ITfContext>,
        wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        let vk = (wparam.0 & 0xff) as u8;

        // 单击 Shift（期间没按别的键）→ 切中英。
        // 必须在抬起时判定：按下时还无法区分「单击」和「Shift+A」。
        // 单击 Shift（按住期间没按过别的键）→ 切中英。
        // 必须在抬起时判定：按下时还无法区分「单击」和「Shift+A」。
        if FeiheTextService::is_shift(vk) {
            let used = self.shift_used.replace(false);
            if !used {
                self.toggle_chinese();
            }
        }
        Ok(BOOL::from(false))
    }

    fn OnPreservedKey(&self, _pic: Ref<'_, ITfContext>, _rguid: *const GUID) -> Result<BOOL> {
        Ok(BOOL::from(false))
    }
}

// ─────────────────────────────────────────────────────────────
// Composition sink
// ─────────────────────────────────────────────────────────────

impl ITfCompositionSink_Impl for FeiheTextService_Impl {
    fn OnCompositionTerminated(
        &self,
        _ec: u32,
        _composition: Ref<'_, ITfComposition>,
    ) -> Result<()> {
        // 系统或宿主应用主动结束了组合（点了别处、应用被关掉等）
        self.composition.borrow_mut().take();
        self.clear_state();
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────
// Edit session：唯一允许写文档的地方
// ─────────────────────────────────────────────────────────────

impl ITfEditSession_Impl for FeiheTextService_Impl {
    fn DoEditSession(&self, ec: u32) -> Result<()> {
        self.run_edit_session(ec)
    }
}

impl FeiheTextService_Impl {
    fn run_edit_session(&self, ec: u32) -> Result<()> {
        let Some(op) = self.pending_op.borrow_mut().take() else {
            return Ok(());
        };
        let Some(context) = self.context.borrow().as_ref().cloned() else {
            self.composition.borrow_mut().take();
            return Ok(());
        };

        unsafe {
            match op {
                EditOp::Start => {
                    if self.begin_composition(&context, ec).is_err() {
                        self.clear_state();
                    }
                }
                EditOp::Update(text) => {
                    // 先把引用 clone 出来，避免 RefCell 借用跨越 EndComposition 之类的重入
                    let comp = self.composition.borrow().as_ref().cloned();
                    if let Some(comp) = comp {
                        let r = comp.GetRange()?;
                        r.SetText(ec, 0, &to_utf16(&text))?;
                        r.Collapse(ec, TF_ANCHOR_END)?;
                        put_caret(&context, ec, &r);
                        self.remember_caret(&context, ec, &r);
                    } else {
                    }
                }
                EditOp::Commit(text) => {
                    // 简繁转换只在这一个出口做：候选里存的永远是词库原文（简体），
                    // 显示层和上屏层各自转。这样切换字形不需要重建任何索引。
                    let text = crate::zhconv::apply(&text);
                    let comp = self.composition.borrow_mut().take();
                    if let Some(comp) = comp {
                        let r = comp.GetRange()?;
                        r.SetText(ec, 0, &to_utf16(&text))?;
                        r.Collapse(ec, TF_ANCHOR_END)?;
                        put_caret(&context, ec, &r);
                        let _ = comp.EndComposition(ec);
                    } else {
                    }
                    self.hide_candidates();
                }
                EditOp::Insert(text) => {
                    // 标点、软键盘符号转换是空操作（它们本来就不在简繁映射里），
                    // 但快捷输入的日期/星期文本要跟着走，所以统一在这里过一遍
                    let text = crate::zhconv::apply(&text);
                    let Ok(range) = caret_range(&context, ec) else {
                        return Ok(());
                    };
                    range.SetText(ec, 0, &to_utf16(&text))?;
                    range.Collapse(ec, TF_ANCHOR_END)?;
                    put_caret(&context, ec, &range);
                }
                EditOp::Cancel => {
                    let comp = self.composition.borrow_mut().take();
                    if let Some(comp) = comp {
                        let r = comp.GetRange()?;
                        r.SetText(ec, 0, &[])?;
                        r.Collapse(ec, TF_ANCHOR_END)?;
                        put_caret(&context, ec, &r);
                        let _ = comp.EndComposition(ec);
                    }
                    self.hide_candidates();
                }
            }
        }
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────
// TSF 小工具
// ─────────────────────────────────────────────────────────────

fn to_utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// 读出并强制打开「中/英」开关。
///
/// `GUID_COMPARTMENT_KEYBOARD_OPENCLOSE` 为 0 时，MSCTF 认为输入法处于英文态，
/// 按键一律放行给应用——这就是「字母直接进输入框、没有候选窗」的根因。
/// 新注册的 TIP 这个值可能是关的，所以激活时直接置 1。
fn open_keyboard(tim: &ITfThreadMgr, tid: u32) {
    let Ok(mgr) = tim.cast::<ITfCompartmentMgr>() else {
        return;
    };
    let Ok(comp) = (unsafe { mgr.GetCompartment(&GUID_COMPARTMENT_KEYBOARD_OPENCLOSE) }) else {
        return;
    };

    let mut v = unsafe { VariantInit() };
    unsafe {
        // VARIANT_0.Anonymous 是 ManuallyDrop，需要显式解引用才能写字段
        let inner = &mut *v.Anonymous.Anonymous;
        inner.vt = VT_I4;
        inner.Anonymous.lVal = 1;
    }
    // 写失败也没别的办法——这里没有能补救的动作，只能让输入法按当前状态跑
    let _ = unsafe { comp.SetValue(tid, &v) };
}

unsafe fn caret_range(context: &ITfContext, ec: u32) -> Result<ITfRange> {
    let mut sel = [TF_SELECTION::default()];
    let mut fetched: u32 = 0;
    context.GetSelection(ec, TF_DEFAULT_SELECTION, &mut sel, &mut fetched)?;
    if fetched == 0 {
        return Err(Error::new(E_FAIL, "no selection"));
    }
    sel[0]
        .range
        .take()
        .ok_or_else(|| Error::new(E_FAIL, "selection has no range"))
}

/// 把插入符放到 range 末尾（range 已 Collapse 到 END）
unsafe fn put_caret(context: &ITfContext, ec: u32, range: &ITfRange) {
    let mut sel = [TF_SELECTION::default()];
    let mut fetched: u32 = 0;
    if context
        .GetSelection(ec, TF_DEFAULT_SELECTION, &mut sel, &mut fetched)
        .is_err()
        || fetched == 0
    {
        return;
    }
    // TF_SELECTION.range 是 ManuallyDrop，数组析构时不会释放里面的引用，
    // 所以两处引用都要手动 take 出来让 Option 自己 drop：
    //   1) GetSelection 返回给我们的那份（由调用方负责释放）
    //   2) 我们 clone 进去、SetSelection 内部会 AddRef 的那份
    drop(sel[0].range.take());
    sel[0].range.replace(range.clone());
    let _ = context.SetSelection(ec, &sel);
    drop(sel[0].range.take());
}

unsafe fn caret_screen_rect(context: &ITfContext, ec: u32, range: &ITfRange) -> Option<RECT> {
    let view = context.GetActiveView().ok()?;
    let mut rc = RECT::default();
    let mut clipped = BOOL::default();
    view.GetTextExt(ec, range, &mut rc, &mut clipped).ok()?;
    Some(rc)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 反引号已经彻底退出键位表，只有测试还要断言「它确实什么都不做」。
    use windows::Win32::UI::Input::KeyboardAndMouse::VK_OEM_3;

    /// 碰 `SCHEME` / `pinyin_mode` 的用例统一拿 dict 模块那把共享锁串行，
    /// 否则和 dict.rs 的测试并行跑会互相篡改（小鹤码在微软方案下查不到）。
    fn lock() -> std::sync::MutexGuard<'static, ()> {
        dict::tests::SCHEME_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// 错音容错只能**追加**，绝不能挤掉或重排正常候选。
    ///
    /// `zhongqing` 在词库里本来就有「钟情」，用户想打的「重庆」挂在 chóng 下面。
    /// 如果容错词抢了 0 号位，正常打字的人按空格就会上屏一个自己没想要的词。
    #[test]
    fn wrong_reading_appends_at_end() {
        let mut list: Vec<Cow<'static, str>> = vec![Cow::Borrowed("钟情")];
        FeiheTextService::append_wrong_reading(&mut list, "zhongqing", true);
        assert_eq!(list[0], "钟情", "正常候选被挤掉了");
        assert!(
            list.iter().skip(1).any(|w| w == "重庆"),
            "没把重庆补进来：{list:?}"
        );

        // 已经在候选里（打对了读音）就不该重复
        let mut dup: Vec<Cow<'static, str>> = vec![Cow::Borrowed("重庆")];
        FeiheTextService::append_wrong_reading(&mut dup, "chongqing", true);
        assert_eq!(dup.len(), 1, "重复追加：{dup:?}");

        // 不相干的码一个都不该加
        let mut none: Vec<Cow<'static, str>> = Vec::new();
        FeiheTextService::append_wrong_reading(&mut none, "nihao", true);
        assert!(none.is_empty(), "乱给了：{none:?}");
    }

    /// 用户报「翻页选字上屏了第一页的字」的那个码，数据层面重现一遍。
    ///
    /// `fwhe` 切成 `[飞][鹤]`，而「鹤」在 `he` 的**第二页第 3 位**（下标 11）。
    /// `confirm_segment` 当年把数字键当页内序号用，按 3 取到的是 `list[2]`「何」，
    /// 也就是第一页那个字。这条测试不测 UI，只钉住「这个场景确实是跨页的」——
    /// 哪天词库变了、「鹤」挪到第一页，这条会失败，提醒我们把验证清单里的手动步骤改掉。
    #[test]
    fn fwhe_splits_with_he_on_second_page() {
        let has = |p: &str| !FeiheTextService::lookup_any(p, 1).is_empty();
        assert_eq!(FeiheTextService::plan_segments("fwhe", has), ["fw", "he"]);

        let he = FeiheTextService::lookup_any("he", 45);
        let pos = he
            .iter()
            .position(|w| w.as_ref() == "鹤")
            .expect("he 的候选里应该有 鹤");
        assert!(
            pos >= candidate::PAGE_SIZE,
            "鹤 应该在第二页（当前下标 {pos}），否则这条用例失去意义"
        );
        // 翻页后按的第 n 个键 → 全表下标 = 页偏移 + n - 1
        let page = pos / candidate::PAGE_SIZE;
        assert_eq!(page, 1, "鹤 应该在第二页");
    }

    /// 用户报的「zuci 只有组词一个候选，想再造祖祠」：判据是候选恰好 1 条、
    /// 且能切成两段。`backspace_into_segments` 就卡这两个条件。
    #[test]
    fn zuci_unique_candidate_is_splittable() {
        let has = |p: &str| !FeiheTextService::lookup_any(p, 1).is_empty();
        assert_eq!(FeiheTextService::lookup_any("zuci", 45).len(), 1, "zuci 应该只有一个候选");
        assert_eq!(FeiheTextService::plan_segments("zuci", has), ["zu", "ci"]);

        // 两段的候选里都得有用户要的字，否则「切成段」也拼不出祖祠
        let zu = FeiheTextService::lookup_any("zu", 45);
        let ci = FeiheTextService::lookup_any("ci", 45);
        assert!(zu.iter().any(|w| w.as_ref() == "祖"), "zu 应该有 祖");
        assert!(ci.iter().any(|w| w.as_ref() == "祠"), "ci 应该有 祠");
    }

    /// 不弹窗模式下候选要拼进组合串：没有候选窗，这是用户唯一能看到候选的地方。
    #[test]
    fn no_popup_hint_lists_current_page() {
        let cands: Vec<Cow<'static, str>> = ["你好", "拟好", "你号"]
            .iter()
            .map(|s| Cow::Borrowed(*s))
            .collect();
        // 前缀是两个空格，序号从 1 开始，中间单空格分隔
        assert_eq!(
            FeiheTextService::format_candidates(&cands, 0),
            "  1.你好 2.拟好 3.你号"
        );
        // 空候选不产生任何尾部（否则组合串后面会挂两个空格）
        assert_eq!(FeiheTextService::format_candidates(&[], 0), "");
        // 翻页后从第 10 条起，序号重新从 1 数——用户按的就是页内序号
        let many: Vec<Cow<'static, str>> = (1..=12)
            .map(|i| Cow::Owned(format!("候选{i}")))
            .collect();
        assert_eq!(
            FeiheTextService::format_candidates(&many, 1),
            "  1.候选10 2.候选11 3.候选12"
        );
    }

    #[test]
    fn code_allowed_rejects_control_chars() {
        let _g = lock();
        config::set_pinyin_mode(false);
        // 组合串只允许编码字符：Ctrl / 退格漏进来时必须被拒
        assert!(FeiheTextService::code_allowed("nihc"));
        assert!(!FeiheTextService::code_allowed("nih\u{1}"));
        assert!(!FeiheTextService::code_allowed("ni\u{8}c"));
        // 快捷输入的引导符 + 字母是合法的
        assert!(FeiheTextService::code_allowed("/rq"));
        assert!(!FeiheTextService::code_allowed("/r1"));
        // 旧的 `\` 引导符已经让给顿号，反引号也不再是编码字符
        assert!(!FeiheTextService::code_allowed("\\rq"));
        assert!(!FeiheTextService::code_allowed("`rq"));
    }

    #[test]
    fn fullwidth_punct_only_for_unshifted_keys() {
        assert_eq!(FeiheTextService::fullwidth_punct(VK_OEM_COMMA.0 as u8), Some('，'));
        assert_eq!(FeiheTextService::fullwidth_punct(VK_OEM_PERIOD.0 as u8), Some('。'));
        // 顿号只留 `\` 一个键（搜狗习惯），`/` 让给快捷输入引导符
        assert_eq!(FeiheTextService::fullwidth_punct(VK_OEM_5.0 as u8), Some('、'));
        assert_eq!(FeiheTextService::fullwidth_punct(VK_OEM_2.0 as u8), None);
        // 反引号既不是引导符也不是间隔号，原样交给应用
        assert_eq!(FeiheTextService::fullwidth_punct(VK_OEM_3.0 as u8), None);
        // 字母键不是标点
        assert_eq!(FeiheTextService::fullwidth_punct(0x41), None);
    }

    #[test]
    fn shift_letter_is_the_uppercase_letter_itself() {
        // Shift+字母：上屏的是大写字母本身，不是编码
        assert_eq!(FeiheTextService::shifted_letter(0x41), Some('A'));
        assert_eq!(FeiheTextService::shifted_letter(0x5A), Some('Z'));
        // 编码路径仍然只认小写：两者必须分工，别哪天又合流
        assert_eq!(FeiheTextService::code_char(0x41), Some('a'));
        // 数字和标点不认：`Shift+1`→`!`、`Shift+/`→`?` 由键盘布局换算，
        // 我们自己按 VK 映射会在非 QWERTY 布局上算错
        for vk in [0x31u8, 0x30, VK_OEM_2.0 as u8, VK_OEM_5.0 as u8, 0x20, 0x0D] {
            assert_eq!(FeiheTextService::shifted_letter(vk), None, "vk=0x{vk:02x}");
        }
    }

    #[test]
    fn page_keys_are_minus_equals_and_arrows() {
        // 中文输入法通行键位：- 上一页 / = 下一页
        assert_eq!(FeiheTextService::page_delta(VK_OEM_MINUS.0 as u8), Some(-1));
        assert_eq!(FeiheTextService::page_delta(VK_OEM_PLUS.0 as u8), Some(1));
        // 方向键保留
        assert_eq!(FeiheTextService::page_delta(0x26), Some(-1));
        assert_eq!(FeiheTextService::page_delta(0x28), Some(1));
        // 字母 / 空格 / 回车 / 左右方向键都不是翻页键
        for vk in [0x41u8, 0x20, 0x0D, 0x25, 0x27] {
            assert_eq!(FeiheTextService::page_delta(vk), None, "vk=0x{vk:02x}");
        }
    }

    #[test]
    fn slash_is_prefix_and_backslash_is_dunhao() {
        // `\` 不再进编码：中文+全角标点时它输出「、」，Shift 组合交给应用出「|」
        assert!(!FeiheTextService::is_code_key(VK_OEM_5.0 as u8));
        assert_eq!(FeiheTextService::code_char(VK_OEM_5.0 as u8), None);
        // `/` 接管快捷输入引导符
        assert!(FeiheTextService::is_code_key(VK_OEM_2.0 as u8));
        assert_eq!(
            FeiheTextService::code_char(VK_OEM_2.0 as u8),
            Some(crate::quick::PREFIX)
        );
        // 反引号彻底退出：既不是引导符也不是顿号
        assert!(!FeiheTextService::is_code_key(VK_OEM_3.0 as u8));
        assert_eq!(FeiheTextService::code_char(VK_OEM_3.0 as u8), None);
    }

    #[test]
    fn bare_prefix_commits_a_slash() {
        // 只敲了引导符本身再回车：中文模式下必须还能打出斜杠
        assert_eq!(FeiheTextService::prefix_literal(false), '/');
        assert_eq!(FeiheTextService::prefix_literal(true), '／');
    }

    /// 一个小词典，模拟「vhsjli = 张三 + 李」但整串无候选的场景
    fn seg_dict(p: &str) -> bool {
        matches!(p, "vhsj" | "li" | "si" | "vh" | "sj")
    }

    #[test]
    fn split_segments_prefers_word_over_single_chars() {
        // 关键用例：vhsjli 既能切成 [vhsj, li]（张三+李），
        // 也能切成 [vh, sj, li]（张+三+李）。必须取前者。
        let r = FeiheTextService::split_segments_units(
            &FeiheTextService::syllables_ds("vhsjli"),
            seg_dict,
        );
        assert_eq!(r, vec!["vhsj", "li"]);
    }

    #[test]
    fn split_segments_three_parts() {
        let r = FeiheTextService::split_segments_units(
            &FeiheTextService::syllables_ds("vhsjlisi"),
            seg_dict,
        );
        assert_eq!(r, vec!["vhsj", "li", "si"]);
    }

    #[test]
    fn split_segments_gives_up_when_unseggable() {
        // 有一段完全匹配不上 → 无法完整切分 → 返回空，让调用方退回别的策略
        let r = FeiheTextService::split_segments_units(
            &FeiheTextService::syllables_ds("vhsjzz"),
            seg_dict,
        );
        assert!(r.is_empty(), "got {r:?}");
        // 少于两个音节不切（调用方只在整串零候选时才进来，这里防呆）
        let r1 = FeiheTextService::split_segments_units(
            &FeiheTextService::syllables_ds("vh"),
            seg_dict,
        );
        assert!(r1.is_empty());
        // vhsj 本身能切成 vh + sj，但真实场景里它整串就有候选，走不到这里
        let r2 = FeiheTextService::split_segments_units(
            &FeiheTextService::syllables_ds("vhsj"),
            seg_dict,
        );
        assert_eq!(r2, vec!["vhsj"]);
    }

    #[test]
    fn split_segments_full_pinyin() {
        let has = |p: &str| matches!(p, "nihao" | "ni" | "hao" | "shi" | "shijie");
        // shijie 有候选，贪心取长 → [nihao, shijie]，而不是 [nihao, shi, jie]
        let units = FeiheTextService::syllables_full("nihaoshijie");
        assert_eq!(units, vec!["ni", "hao", "shi", "jie"]);
        assert_eq!(
            FeiheTextService::split_segments_units(&units, has),
            vec!["nihao", "shijie"]
        );
        // 没有 shijie 时才退到单音节
        let has2 = |p: &str| matches!(p, "nihao" | "ni" | "hao" | "shi" | "jie");
        assert_eq!(
            FeiheTextService::split_segments_units(&units, has2),
            vec!["nihao", "shi", "jie"]
        );
        // 有一段彻底没候选（jie 不在词库）→ 整串切不动，返回空
        let has3 = |p: &str| matches!(p, "nihao" | "ni" | "hao" | "shi");
        assert!(FeiheTextService::split_segments_units(&units, has3).is_empty());
    }

    #[test]
    fn split_pinyin_handles_variable_length_syllables() {
        // 全拼音节不定长：ni(2) / hao(3) / zhuang(6) / a(1)
        assert_eq!(
            FeiheTextService::split_pinyin("nihaozhuang").unwrap(),
            vec!["ni", "hao", "zhuang"]
        );
        // 长音节优先：xian 不切成 xi + an
        assert_eq!(FeiheTextService::split_pinyin("xian").unwrap(), vec!["xian"]);
        assert_eq!(
            FeiheTextService::split_pinyin("fangan").unwrap(),
            vec!["fang", "an"]
        );
        // 切不出来（不是合法拼音）→ None，调用方据此「再等等」
        assert!(FeiheTextService::split_pinyin("zzz").is_none());
        assert!(FeiheTextService::split_pinyin("").is_none());
    }

    #[test]
    fn manual_separator_forces_boundary() {
        // 全拼：ni'hao 强制断在 ni 后面，即使 nihao 本身能整串切
        assert_eq!(
            FeiheTextService::syllables_full("ni'hao"),
            vec!["ni", "hao"]
        );
        // xian 默认整串，加分隔符后强制拆开
        assert_eq!(FeiheTextService::syllables_full("xi'an"), vec!["xi", "an"]);
        // 双拼：分隔符同样生效（音节定长，位置不动）
        assert_eq!(
            FeiheTextService::syllables_ds("vh'sjli"),
            vec!["vh", "sj", "li"]
        );
        // 双拼奇数分组 → 音节没打完，整串放弃
        assert!(FeiheTextService::syllables_ds("vh's").is_empty());
        // 结尾/连续分隔符被忽略
        assert_eq!(FeiheTextService::syllables_ds("vhsj'"), vec!["vh", "sj"]);
    }

    /// 长句分段决策。`viuiqi`（指示器）这种「整串没候选」的输入，
    /// 必须切成多段让用户逐段选，**不能替他上屏**。
    #[test]
    fn plan_segments_splits_long_input() {
        // 假词典：只要 `viui`(只是) 和 `qi`(其) 有词
        let has = |p: &str| matches!(p, "viui" | "qi");
        assert_eq!(FeiheTextService::plan_segments("viuiqi", has), vec!["viui", "qi"]);

        // 音节没打完（双拼奇数长度）→ 空，调用方「再等等」
        assert!(FeiheTextService::plan_segments("viuiq", has).is_empty());

        // 单音节不拆
        assert!(FeiheTextService::plan_segments("vi", has).is_empty());
    }

    /// DP 要求「每段都能出候选」，某个音节词库里完全没有时会整串失败。
    /// 这时退到逐音节——可能有个别段空着，但至少看得见、能退格，不替他上屏。
    #[test]
    fn plan_segments_falls_back_to_single_syllables() {
        // `ui` 查不到，`vi`/`qi` 有 → DP 覆盖不了整串
        let has = |p: &str| matches!(p, "vi" | "qi");
        assert_eq!(
            FeiheTextService::plan_segments("viuiqi", has),
            vec!["vi", "ui", "qi"]
        );
    }

    /// 整串本身就是一个「词」（自造词、超长词条）时，也必须能拆成逐音节。
    ///
    /// 这是「同码第二个自造词造不出来」的回归测试：`djdjth` 学会「蛋蛋堂」之后，
    /// 这个码整串就有候选，DP 贪心会取最长（整串一段），左右键进分词模式后
    /// 只能看到一段、退格直接退音节。`plan_segments` 见到「只切出一段」必须
    /// 退回逐音节，否则逐字选择这条路就断了。
    #[test]
    fn plan_segments_splits_whole_code_word_into_syllables() {
        // 假词典：只有整串这个词，单音节也都有字（真实情况就是这样）
        let has = |p: &str| matches!(p, "djdjth" | "dj" | "th");
        assert_eq!(
            FeiheTextService::plan_segments("djdjth", has),
            vec!["dj", "dj", "th"],
            "整串是一个词时也要拆到逐音节"
        );
    }

    /// 分段不能吞字：拼回去必须还是原编码。
    #[test]
    fn plan_segments_preserves_every_char() {
        let has = |p: &str| matches!(p, "vhsj" | "li" | "vh" | "sj");
        let segs = FeiheTextService::plan_segments("vhsjli", has);
        assert!(segs.len() >= 2, "got {segs:?}");
        assert_eq!(segs.concat(), "vhsjli");
    }

    #[test]
    fn merge_segments_keeps_empty_segment_visible() {
        let _g = lock();
        config::set_pinyin_mode(false);
        dict::set_scheme(Scheme::Xiaohe);
        let seg = |code: &str, word: &str| Segment {
            code: code.to_string(),
            word: word.to_string(),
            picked: false,
        };
        // 正常段：直接拼已选的词
        assert_eq!(
            FeiheTextService::merge_segments(&[seg("vhsj", "张三"), seg("li", "李")]),
            "张三李"
        );
        // 空段但段码查得到：补查回来，不能吞
        let first = FeiheTextService::lookup_any("li", 1)
            .first()
            .map(|s| s.to_string())
            .expect("li 至少有一个候选");
        let got = FeiheTextService::merge_segments(&[seg("vhsj", "张三"), seg("li", "")]);
        assert_eq!(got, format!("张三{first}"));
        // 空段且段码也查不到：上屏段码本身，至少让用户看见敲了什么
        // （段码得现找——`zzzz` 这种看着像乱码的其实是合法双拼码，能查出「走走」）
        let dead = ["qqqq", "vvvv", "bbbb", "pppp", "mmmm"]
            .into_iter()
            .find(|c| FeiheTextService::lookup_any(c, 1).is_empty())
            .expect("至少要有一个查不到候选的段码");
        let got = FeiheTextService::merge_segments(&[seg("vhsj", "张三"), seg(dead, "")]);
        assert_eq!(got, format!("张三{dead}"));
    }

    #[test]
    fn query_code_strips_separator() {
        assert_eq!(FeiheTextService::query_code("ni'hao").as_ref(), "nihao");
        assert_eq!(FeiheTextService::query_code("vhsj").as_ref(), "vhsj");
    }

    /// 「先连打、再按 Tab」：末尾的形码该拆出去，但**不能拆掉半个音节**。
    ///
    /// 用户是拿自然码 / 小鹤的习惯先把整个码敲完（`ddj`）再按 Tab 的，老行为下
    /// 这一按只是「进入辅助码态但辅助码为空」，接着敲的 `j` 会跟 `ddj` 拼成前缀，
    /// 查了个空——用户看到的就是「辅助码对我没用」。
    #[test]
    fn split_trailing_assist_takes_shape_keys() {
        let always = |_: &str, _: &str| true;
        // 自然码 1 键：dd + j
        assert_eq!(
            FeiheTextService::split_trailing_assist("ddj", 1, always),
            Some(("dd".into(), "j".into()))
        );
        // 小鹤 2 键：先试 2 键，成了就不再多试
        assert_eq!(
            FeiheTextService::split_trailing_assist("ddj", 2, |h, t| h == "dd" && t == "j"),
            Some(("dd".into(), "j".into()))
        );
        assert_eq!(
            FeiheTextService::split_trailing_assist("ddfj", 2, always),
            Some(("dd".into(), "fj".into()))
        );
    }

    /// 完码不能被拆：`de` 是按 Tab 想筛「的」，不是把 `e` 当辅助码。
    /// 判据是「剥完剩下的前缀长度必须是偶数」——双拼每个音节固定两键。
    #[test]
    fn split_trailing_assist_keeps_whole_syllable() {
        let always = |_: &str, _: &str| true;
        assert_eq!(FeiheTextService::split_trailing_assist("de", 1, always), None);
        assert_eq!(FeiheTextService::split_trailing_assist("de", 2, always), None);
        // 单键码更不可能拆
        assert_eq!(FeiheTextService::split_trailing_assist("d", 2, always), None);
        // 手动分隔符 / 非字母：不碰
        assert_eq!(FeiheTextService::split_trailing_assist("dd'j", 1, always), None);
        assert_eq!(FeiheTextService::split_trailing_assist("dd1", 1, always), None);
        // 剥了也筛不出东西，就别剥（用户可能只是打错了一个键）
        assert_eq!(
            FeiheTextService::split_trailing_assist("ddj", 1, |_, _| false),
            None
        );
    }

    /// 真表验证：`dd` + `j`（自然码，部首 巾）确实能筛出「带」，
    /// `dd` + `fj`（小鹤，带上 + 巾）也能。这条把「拆出来的码真的有用」钉住——
    /// 上面几条只用假闭包，码表坏了它们照样过。
    #[test]
    fn split_trailing_assist_hits_real_table() {
        let _g = dict::tests::SCHEME_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        dict::set_scheme(Scheme::Xiaohe);
        for (mode, code) in [
            (AssistMode::Ziranma, "ddj"),
            (AssistMode::Xiaohe, "ddfj"),
        ] {
            let got = FeiheTextService::split_trailing_assist(code, mode.max_keys(), |p, a| {
                FeiheTextService::assist_hit(p, a, mode)
            });
            assert!(got.is_some(), "{mode:?} 的 {code} 应该能拆");
            let (_, assist) = got.unwrap();
            let hit = dict::lookup_filtered("dd", &assist, mode, 45);
            assert!(
                hit.iter().any(|w| &**w == "带"),
                "{mode:?} dd + {assist} 应该筛出「带」，实际 {hit:?}"
            );
        }
    }

    /// 「3 键直出」只认 3 键：4 键 `ddfj` 是小鹤的正常码长（2 音节 + 2 形码），
    /// 2 键是完整音节，5 键可能是两个半音节——越长的码越不该替用户猜。
    #[test]
    fn auto_assist_split_only_three_keys() {
        let always = |_: &str, _: &str| true;
        assert_eq!(
            FeiheTextService::auto_assist_split("ddj", 1, always),
            Some(("dd".into(), "j".into()))
        );
        assert_eq!(FeiheTextService::auto_assist_split("ddfj", 2, always), None);
        assert_eq!(FeiheTextService::auto_assist_split("dd", 1, always), None);
        assert_eq!(FeiheTextService::auto_assist_split("dj", 1, always), None);
        // 拆出来也筛不出东西就别拆——第 3 键原样留在编码里
        assert_eq!(
            FeiheTextService::auto_assist_split("ddj", 1, |_, _| false),
            None
        );
    }

    /// 真表验证「筛出单字」这道门槛：`dd` + `j`（自然码，部首 巾）第一项是单字，
    /// 所以 3 键直出能落地；这条同时钉住 `ddj` → 「带」。
    #[test]
    fn auto_assist_split_hits_single_char() {
        let _g = dict::tests::SCHEME_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        dict::set_scheme(Scheme::Xiaohe);
        let mode = AssistMode::Ziranma;
        let got = FeiheTextService::auto_assist_split("ddj", mode.max_keys(), |p, a| {
            FeiheTextService::assist_hit_single(p, a, mode)
        });
        assert_eq!(got, Some(("dd".into(), "j".into())));

        let hit = dict::lookup_filtered("dd", "j", mode, 1);
        assert!(
            hit.first().is_some_and(|w| w.chars().count() == 1),
            "第一位必须是单字，否则 3 键直出会吞掉「代价」这类词：{hit:?}"
        );
        let wide = dict::lookup_filtered("dd", "j", mode, 45);
        assert!(wide.iter().any(|w| &**w == "带"), "dd + j 应该筛出「带」");
    }

    /// 自造词的准入规则：`li`+`mk`+`yr` → 李明远，四道门全过才记。
    #[test]
    fn compose_phrase_learns_new_word() {
        let seg = |code: &str, word: &str| Segment {
            code: code.to_string(),
            word: word.to_string(),
            picked: false,
        };
        let segs = vec![seg("li", "李"), seg("mk", "明"), seg("yr", "远")];
        // 整串查不到（词库里没有「李明远」），单段能查到全拼
        let lookup = |code: &str, word: &str| match (code, word) {
            ("limkyr", _) => None,
            ("li", "李") => Some("li".to_string()),
            ("mk", "明") => Some("ming".to_string()),
            ("yr", "远") => Some("yuan".to_string()),
            _ => None,
        };
        let got = FeiheTextService::compose_phrase(&segs, "limkyr", lookup);
        assert_eq!(
            got,
            Some(("李明远".to_string(), "li ming yuan".to_string()))
        );
    }

    /// 回车 = 字母；但用户确认过的段保留汉字，不能连人带字一起丢掉。
    ///
    /// 这条是「输入 `mmap` 回车却出「面ap」」的回归测试。
    #[test]
    fn raw_text_keeps_only_picked_segments() {
        let mk = |code: &str, word: &str, picked: bool| Segment {
            code: code.into(),
            word: word.into(),
            picked,
        };
        // 一段都没确认：整串原样回落成字母
        let segs = vec![mk("mm", "面", false), mk("ap", "", false)];
        assert_eq!(FeiheTextService::raw_text(&segs), "mmap");
        // 确认过第一段：「面」是他选的，后面的字母照原样
        let segs = vec![mk("mm", "面", true), mk("ap", "", false)];
        assert_eq!(FeiheTextService::raw_text(&segs), "面ap");
        // 确认过但词是空的（理论上不会发生）：退回编码，绝不产出半个空串
        let segs = vec![mk("mm", "", true)];
        assert_eq!(FeiheTextService::raw_text(&segs), "mm");
        // 全部确认过：等价于整体上屏
        let segs = vec![mk("limk", "李明", true), mk("yr", "远", true)];
        assert_eq!(FeiheTextService::raw_text(&segs), "李明远");
    }

    /// 四个「不该记」的条件，逐个验一遍。
    #[test]
    fn compose_phrase_rejects_junk() {
        let seg = |code: &str, word: &str| Segment {
            code: code.to_string(),
            word: word.to_string(),
            picked: false,
        };
        let all = |_: &str, _: &str| -> Option<String> { Some("li ming yuan".into()) };

        // 1. 只有一段：单字不叫组词
        let one = vec![seg("li", "李")];
        assert_eq!(FeiheTextService::compose_phrase(&one, "li", all), None);

        // 2. 有空段：那段是兜底塞的段码，不是用户选的
        let empty = vec![seg("li", "李"), seg("mk", ""), seg("yr", "远")];
        assert_eq!(FeiheTextService::compose_phrase(&empty, "limkyr", all), None);

        // 3. 段码拼起来和用户敲的对不上（编码中途改过）
        let mismatch = vec![seg("li", "李"), seg("mk", "明")];
        assert_eq!(
            FeiheTextService::compose_phrase(&mismatch, "limkyr", all),
            None
        );

        // 4. 词库里本来就有：查得到整串就跳过（自动切分沿用默认不该记）
        let known = vec![seg("li", "李"), seg("mk", "明"), seg("yr", "远")];
        let in_dict = |_: &str, _: &str| -> Option<String> { Some("li ming yuan".into()) };
        assert_eq!(
            FeiheTextService::compose_phrase(&known, "limkyr", in_dict),
            None
        );

        // 5. 有一段拿不到全拼：整体放弃，宁可这次不记，也不记个残缺的
        let segs = vec![seg("li", "李"), seg("mk", "明"), seg("yr", "远")];
        let partial = |code: &str, word: &str| -> Option<String> {
            match (code, word) {
                ("limkyr", _) => None,
                ("li", "李") => Some("li".into()),
                ("mk", "明") => None,
                _ => Some("yuan".into()),
            }
        };
        let got = FeiheTextService::compose_phrase(&segs, "limkyr", partial);
        assert_eq!(got, None);
    }

    #[test]
    fn can_insert_sep_guards() {
        let _g = lock();
        config::set_pinyin_mode(false);
        dict::set_scheme(Scheme::Xiaohe);
        // 空编码 / 连续分隔符 / 半截音节都不接受
        assert!(!FeiheTextService::can_insert_sep(""));
        assert!(!FeiheTextService::can_insert_sep("vhsj'"));
        assert!(!FeiheTextService::can_insert_sep("vhs"));
        assert!(!FeiheTextService::can_insert_sep("`rq"));
        // 完整音节之后可以
        assert!(FeiheTextService::can_insert_sep("vhsj"));
        // 全拼不限制位置（音节不定长，交给 DP 判断）
        config::set_pinyin_mode(true);
        assert!(FeiheTextService::can_insert_sep("niha"));
        config::set_pinyin_mode(false);
    }

    #[test]
    fn brief_alt_only_for_single_letter_in_shuangpin() {
        let _g = lock();
        dict::set_scheme(Scheme::Xiaohe);
        config::set_pinyin_mode(false);
        assert_eq!(FeiheTextService::brief_alt("d"), Some("打"));
        assert_eq!(FeiheTextService::brief_alt("c"), Some("词库"));
        assert_eq!(FeiheTextService::brief_alt("de"), None, "只有单键才有次选");
        assert_eq!(FeiheTextService::brief_alt(""), None);
        // 全拼下一个字母后面还能跟分隔符（`a'i`），分号不能挪作次选键
        config::set_pinyin_mode(true);
        assert_eq!(FeiheTextService::brief_alt("d"), None);
        config::set_pinyin_mode(false);
    }

    #[test]
    fn split_segments_real_dict() {
        let _g = lock();
        dict::set_scheme(Scheme::Xiaohe);
        config::set_pinyin_mode(false);
        for c in ["nihcui", "womfui", "vhsjui", "vhsjlisi", "vhsjzz", "nihczz",
                  "uijmgo", "vsgouijm", "nihaoshijie"] {
            let segs = FeiheTextService::split_segments(c);
            let has_cand = !dict::lookup(c, 1).is_empty();
            println!("{c:<12} cand={has_cand:<5} segs={segs:?}");
        }

        // 全拼走另一套索引（lookup_pinyin），单独跑一遍确认没接错
        config::set_pinyin_mode(true);
        for c in ["nihaoshijie", "woyaochifan", "xianzai", "ni'hao", "zhongguoren"] {
            let segs = FeiheTextService::split_segments(c);
            let has_cand = !dict::lookup_pinyin(c, 1).is_empty();
            println!("{c:<12} cand={has_cand:<5} segs={segs:?}");
        }
        config::set_pinyin_mode(false);
    }

    /// 用户实际反馈的串：`limkyr`（李明远）整串无候选时，自动切成 `李明|远`。
    ///
    /// 这里用假词典固定切法，不依赖真实词库里是否恰好有整串能命中的词——
    /// 那种词会走「有候选、不分词」的另一条路，就测不到分段本身了。
    #[test]
    fn limkyr_splits_into_word_plus_char() {
        let _g = lock();
        config::set_pinyin_mode(false);
        let units = FeiheTextService::syllables_ds("limkyr");
        assert_eq!(units, ["li", "mk", "yr"]);
        let r = FeiheTextService::split_segments_units(&units, |p| {
            matches!(p, "li" | "mk" | "yr" | "limk")
        });
        assert_eq!(r, vec!["limk", "yr"]);
    }

    #[test]
    fn expand_segment_splits_word_into_single_chars() {
        let _g = lock();
        dict::set_scheme(Scheme::Xiaohe);
        config::set_pinyin_mode(false);
        let mut segs = vec![
            Segment {
                code: "limk".into(),
                word: "李明".into(),
                picked: false,
            },
            Segment {
                code: "yr".into(),
                word: "远".into(),
                picked: false,
            },
        ];
        assert!(FeiheTextService::expand_segment(&mut segs, 0));
        let codes: Vec<&str> = segs.iter().map(|s| s.code.as_str()).collect();
        assert_eq!(codes, vec!["li", "mk", "yr"]);
        // 拆出来的每段都要有默认候选，不能是空词
        assert!(segs.iter().all(|s| !s.word.is_empty()), "{segs:?}");
        // 原来那个「李明」已经被拆掉了，不能残留在第一段
        assert_ne!(segs[0].word, "李明");
    }

    #[test]
    fn expand_segment_refuses_single_syllable() {
        let _g = lock();
        dict::set_scheme(Scheme::Xiaohe);
        config::set_pinyin_mode(false);
        let mut segs = vec![
            Segment {
                code: "limk".into(),
                word: "李明".into(),
                picked: false,
            },
            Segment {
                code: "yr".into(),
                word: "远".into(),
                picked: false,
            },
        ];
        // 第 1 段本来就是单音节 → 拆不动，调用方应该退而去删音节
        assert!(!FeiheTextService::expand_segment(&mut segs, 1));
        assert_eq!(segs.len(), 2);
        // 越界也不能 panic（组合态可能刚好被宿主打断）
        assert!(!FeiheTextService::expand_segment(&mut segs, 9));
    }

    #[test]
    fn pick_expand_target_prefers_cursor_then_last() {
        let _g = lock();
        config::set_pinyin_mode(false);
        let seg = |code: &str, word: &str| Segment {
            code: code.into(),
            word: word.into(),
            picked: false,
        };
        // 光标在多音节段上 → 拆它
        let segs = vec![seg("limk", "李明"), seg("yr", "远")];
        assert_eq!(FeiheTextService::pick_expand_target(&segs, 0), Some(0));
        // 光标停在单字上 → 拆最后一个还能拆的段（退格是从右往左）
        assert_eq!(FeiheTextService::pick_expand_target(&segs, 1), Some(0));
        // 全是单字 → 拆不动，交给删音节
        let segs = vec![seg("li", "李"), seg("mk", "明"), seg("yr", "远")];
        assert_eq!(FeiheTextService::pick_expand_target(&segs, 2), None);
        // 越界索引不 panic
        assert_eq!(FeiheTextService::pick_expand_target(&segs, 9), None);
    }

    #[test]
    fn drop_last_syllable_goes_by_syllable() {
        let _g = lock();
        config::set_pinyin_mode(false);
        // 一次退一个音节（2 键），不会停在 `limky` 这种半截串上
        assert_eq!(FeiheTextService::drop_last_syllable("limkyr"), "limk");
        assert_eq!(FeiheTextService::drop_last_syllable("limk"), "li");
        assert_eq!(FeiheTextService::drop_last_syllable("li"), "");
        // 分隔符先单独退掉，不连音节一起吞
        assert_eq!(FeiheTextService::drop_last_syllable("ni'hc"), "ni");
        assert_eq!(FeiheTextService::drop_last_syllable("ni'"), "ni");
        // 半截音节（还没打完）退一个字符，保证退格永远有反应
        assert_eq!(FeiheTextService::drop_last_syllable("limky"), "limk");
        assert_eq!(FeiheTextService::drop_last_syllable("l"), "");
    }

    /// 零声母词的可达性回归。
    ///
    /// 小鹤的零声母按韵母长度分三种，**双字母韵母保持全拼形式**：
    /// `an` → `an`（不是 `aj`）、`ai` → `ai`、`ou` → `ou`。
    /// 所以「按钮」= `annq`。
    ///
    /// 这条早期实现错了（一刀切按「首字母 + 韵母键」，把 `an` 算成 `aj`），
    /// 用户拿官方反查工具对出来才发现的。用测试把正确码钉死，别再漂回去。
    #[test]
    fn zero_initial_words_are_reachable() {
        let _g = lock();
        dict::set_scheme(Scheme::Xiaohe);
        config::set_pinyin_mode(false);

        assert_eq!(FeiheTextService::syllables_ds("annq"), vec!["an", "nq"]);

        // 硬编码一个，把「零声母 an → an」这条用户可见规则钉死
        let list = dict::lookup("annq", 30);
        assert!(
            list.iter().any(|w| w.as_ref() == "按钮"),
            "annq 应出「按钮」，实际候选 {:?}",
            list.iter().take(6).collect::<Vec<_>>()
        );

        // 其余走编码器，顺便校验「编码器 ↔ 词库索引」是一致的
        for (py, word) in [
            ("an quan", "安全"),
            ("an shi", "按时"),
            ("ai qing", "爱情"),
            ("ou zhou", "欧洲"),
            ("e xin", "恶心"),
        ] {
            let code = shuangpin::encode_pinyin(Scheme::Xiaohe, py).expect(py);
            let list = dict::lookup(&code, 30);
            assert!(
                list.iter().any(|w| w.as_ref() == word),
                "{py} -> {code} 应出「{word}」，实际候选 {:?}",
                list.iter().take(6).collect::<Vec<_>>()
            );
        }
    }

    /// 用户核心诉求：**词库里没有这个词，也得让我自己一个字一个字组出来**。
    ///
    /// 机制是 `maybe_split` 的分词模式：整串零候选时把编码切成音节段，
    /// 每段单独给候选，用户 `→` 移段、数字换字。
    /// 这里用「词库确实没有整词」的组合验证切段路径成立。
    ///
    /// 码一律由 `encode_pinyin` 生成，不手写——手写双拼码太容易错
    /// （踩过：`宾` 是 `bb` 不是 `bv`，`bv` 其实是 `bui`，非法）。
    #[test]
    fn can_compose_char_by_char_without_the_whole_word() {
        let _g = lock();
        dict::set_scheme(Scheme::Xiaohe);
        config::set_pinyin_mode(false);

        let has = |p: &str| !dict::lookup(p, 1).is_empty();
        let mut checked = 0;
        for py in [
            "an bin", "an ping", "an di", "niu an", "niu de", "niu ge", "e xin", "ou zhou",
        ] {
            let code = shuangpin::encode_pinyin(Scheme::Xiaohe, py).expect(py);
            if !dict::lookup(&code, 1).is_empty() {
                continue; // 词库里恰好有整词，跳过（本用例只关心「没有整词」的场景）
            }
            let units = FeiheTextService::syllables_ds(&code);
            assert_eq!(units.len(), 2, "{py} -> {code} 应切成 2 个音节");
            // 每个音节自己都得能出单字，否则「组词」无从谈起
            for u in &units {
                assert!(has(u), "{py} -> {code} 的音节 {u} 查不到单字");
            }
            let segs = FeiheTextService::split_segments_units(&units, has);
            assert_eq!(segs, units, "{py} -> {code} 应能逐字组词，实际切段 {segs:?}");
            checked += 1;
        }
        assert!(checked >= 3, "有效样本太少（{checked}），用例失去意义");
    }

    /// 常用词可达性抽查：编码器算出来的码，词库必须查得到。
    ///
    /// 这类问题最难自查——用户只会觉得「打不出来」，不会想到是词库缺词。
    /// 之前 `iuliqi` 出「处理器吧」就是词库缺「处理器」。词缺了就往
    /// `src/extra_dict.txt` 里补，这个用例负责把缺口暴露出来。
    #[test]
    fn common_words_are_reachable() {
        let _g = lock();
        dict::set_scheme(Scheme::Xiaohe);
        config::set_pinyin_mode(false);

        const WORDS: &[(&str, &str)] = &[
            ("按钮", "an niu"),
            ("安全", "an quan"),
            ("按时", "an shi"),
            ("处理器", "chu li qi"),
            ("显示器", "xian shi qi"),
            ("笔记本", "bi ji ben"),
            ("输入法", "shu ru fa"),
            ("快捷键", "kuai jie jian"),
            ("文件夹", "wen jian jia"),
            ("浏览器", "liu lan qi"),
            ("内存条", "nei cun tiao"),
            ("摄像头", "she xiang tou"),
            ("二维码", "er wei ma"),
            ("验证码", "yan zheng ma"),
            ("数据库", "shu ju ku"),
            ("服务器", "fu wu qi"),
            ("客户端", "ke hu duan"),
            ("登录", "deng lu"),
            ("注销", "zhu xiao"),
            ("刷新", "shua xin"),
            ("重启", "chong qi"),
            ("截图", "jie tu"),
            ("复制", "fu zhi"),
            ("粘贴", "zhan tie"),
            ("搜索", "sou suo"),
            ("下载", "xia zai"),
            ("上传", "shang chuan"),
            ("安装", "an zhuang"),
            ("卸载", "xie zai"),
            ("更新", "geng xin"),
            ("配置", "pei zhi"),
            ("参数", "can shu"),
            ("编译", "bian yi"),
            ("调试", "tiao shi"),
            ("部署", "bu shu"),
            ("依赖", "yi lai"),
            ("兼容", "jian rong"),
            ("性能", "xing neng"),
            ("延迟", "yan chi"),
            ("体积", "ti ji"),
        ];

        let mut missing = Vec::new();
        for (word, py) in WORDS {
            let code = shuangpin::encode_pinyin(Scheme::Xiaohe, py).expect(py);
            let list = dict::lookup(&code, 30);
            if !list.iter().any(|w| w.as_ref() == *word) {
                missing.push(format!("{word}({py}) -> {code}"));
            }
        }
        assert!(
            missing.is_empty(),
            "以下常用词查不到，请补进 src/extra_dict.txt：\n  {}",
            missing.join("\n  ")
        );
    }

    #[test]
    fn rare_name_chars_are_reachable() {
        let _g = lock();
        dict::set_scheme(Scheme::Xiaohe);
        config::set_pinyin_mode(false);

        // 姓名 / 地名里的生僻字。它们词频很低，但**缺了就是打不出来**。
        //
        // 这是踩过的坑：早期 gen_dict.py 按词频把单字截断到 3500 个，某个生僻姓
        // 在 8105 字表里排 5402 名（词频 796，而第 3500 名的下限是 11418），被直接
        // 截掉，翻页也翻不到。现在单字全收（7896 个），这里钉死。
        const NAMES: &[(&str, &str)] = &[
            ("仝", "tong"),
            ("佟", "tong"),
            ("覃", "qin"),
            ("翟", "zhai"),
            ("邬", "wu"),
            ("阚", "kan"),
            ("亓", "qi"),
            ("郗", "xi"),
            ("禚", "zhuo"),
            ("仉", "zhang"),
        ];

        let mut missing = Vec::new();
        for (word, py) in NAMES {
            let code = shuangpin::encode_pinyin(Scheme::Xiaohe, py).expect(py);
            // 用 MAX_CANDIDATES（候选窗能翻到的上限），不是随手取 10
            let list = dict::lookup(&code, MAX_CANDIDATES);
            if !list.iter().any(|w| w.as_ref() == *word) {
                missing.push(format!("{word}({py}) -> {code}"));
            }
        }
        assert!(
            missing.is_empty(),
            "以下生僻姓名用字查不到（单字不该按词频截断）：\n  {}",
            missing.join("\n  ")
        );
    }

    #[test]
    fn is_shift_recognizes_all_variants() {
        assert!(FeiheTextService::is_shift(0x10));
        assert!(FeiheTextService::is_shift(0xA0));
        assert!(FeiheTextService::is_shift(0xA1));
        assert!(!FeiheTextService::is_shift(0x41));
        assert!(!FeiheTextService::is_shift(0x0D));
    }
}
