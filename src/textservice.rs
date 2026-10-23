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

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Variant::{VariantInit, VARIANT, VT_I4};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_OEM_1, VK_OEM_2, VK_OEM_3, VK_OEM_4, VK_OEM_5,
    VK_OEM_6, VK_OEM_7, VK_OEM_COMMA, VK_OEM_MINUS, VK_OEM_PERIOD, VK_OEM_PLUS, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::TextServices::*;
use windows_core::{implement, IUnknownImpl};

use std::borrow::Cow;
use std::cell::{Cell, RefCell};

/// 一次最多取多少候选（10 页 × 9）
const MAX_CANDIDATES: usize = 90;

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
            candidates: RefCell::new(Vec::new()),
            selected: Cell::new(0),
            page: Cell::new(0),
            caret_x: Cell::new(0),
            caret_y: Cell::new(0),
            pending_op: RefCell::new(None),
            shift_used: Cell::new(false),
            segments: RefCell::new(Vec::new()),
            seg_idx: Cell::new(0),
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
        self.segments.borrow_mut().clear();
        self.seg_idx.set(0);
        self.candidates.borrow_mut().clear();
        self.selected.set(0);
        self.page.set(0);
        self.pending_op.borrow_mut().take();
        candidate::hide();
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

    /// 把分段选择拼成上屏文本。纯函数，理由同 `plan_segments`。
    ///
    /// 空段必须兜底：`plan_segments` 退到逐音节时，个别音节可能查不到候选，
    /// 直接拼 `word` 会把它**静默吞掉**——用户敲了 3 个音节只得到 2 个字，
    /// 而且屏幕上没有任何提示。先按段码再查一次，仍然没有就上屏段码本身
    /// （和 `commit_raw` 一个口径：宁可让用户看见自己敲了什么）。
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
    /// `\` → `、` 是照搜狗的习惯（微软拼音把 `、` 放在 `/` 上）。两个键都给 `、`
    /// 不冲突，反而是两拨用户都能用。注意 `\` 原来是快捷输入的引导符，引导符
    /// 已经挪到 `` ` ``（见 `is_code_key`）——不然 `、` 根本打不出来。
    fn fullwidth_punct(vk: u8) -> Option<char> {
        let ch = match vk {
            v if v == VK_OEM_COMMA.0 as u8 => '，',
            v if v == VK_OEM_PERIOD.0 as u8 => '。',
            v if v == VK_OEM_1.0 as u8 => '；',
            v if v == VK_OEM_2.0 as u8 => '、',
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
    /// 另外都接受手动分隔键（见 `is_separator_key`）和快捷输入的引导符 `` ` ``。
    fn is_code_key(vk: u8) -> bool {
        if vk.is_ascii_uppercase() {
            return true;
        }
        // Shift 组合不参与编码：`Shift+\` 要输出 `|`，`Shift+`` ` `` 要输出 `~`，
        // 这些都得原样交给应用。字母不受影响（上面已经返回 true 了）。
        if Self::shift_down() {
            return false;
        }
        // 快捷输入的引导符 `` ` ``
        if vk == VK_OEM_3.0 as u8 {
            return true;
        }
        // 微软双拼的 `ing` 在分号键
        if vk == VK_OEM_1.0 as u8 && !config::pinyin_mode() && dict::scheme() == Scheme::Ms {
            return true;
        }
        // 手动分隔
        Self::is_separator_key(vk)
    }

    /// 虚拟键 → 编码字符
    fn code_char(vk: u8) -> Option<char> {
        if vk.is_ascii_uppercase() {
            return Some((vk as char).to_ascii_lowercase());
        }
        if vk == VK_OEM_3.0 as u8 {
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
        // 快捷输入：引导符（反引号，见 quick::PREFIX）+ 纯小写字母
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

    /// 重新查候选并回到第一页第一项
    fn refresh(&self) {
        // 分词模式：候选就是当前这一段的
        if !self.segments.borrow().is_empty() {
            let idx = self.seg_idx.get().min(self.segments.borrow().len() - 1);
            let code = self.segments.borrow()[idx].code.clone();
            let word = self.segments.borrow()[idx].word.clone();
            let list = FeiheTextService::lookup_any(&code, MAX_CANDIDATES);
            // 选中项对齐到这一段当前选中的词，左右移动时不会跳回第一项
            let sel = list.iter().position(|w| &**w == word).unwrap_or(0);
            self.candidates.borrow_mut().clone_from(&list);
            self.selected.set(sel);
            self.page.set(0);
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
        let list = if config::pinyin_mode() {
            // 全拼模式：前缀是「全拼去掉空格」的串（nihao），走另一套索引
            dict::lookup_pinyin_filtered(query, &assist_prefix, mode, MAX_CANDIDATES)
        } else if assist_prefix.is_empty() {
            dict::lookup(query, MAX_CANDIDATES)
        } else {
            dict::lookup_filtered(query, &assist_prefix, mode, MAX_CANDIDATES)
        };
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
            return s;
        }
        let code = self.code.borrow();
        let assist = self.assist.borrow();
        if assist.is_empty() {
            code.clone()
        } else {
            format!("{code} {assist}")
        }
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
            crate::trace!("start_composition: rejected code {code:?}");
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
        crate::trace!("chinese -> {next}");
    }

    fn commit_current(&self) {
        self.commit_with("");
    }

    /// 提交当前候选，并在后面粘上一段文本（通常是全角标点）。
    ///
    /// 必须合成**一个** `EditOp`：`pending_op` 只有一个槽位，连着发 `Commit` 和
    /// `Insert` 会把前一个冲掉，结果是字没上屏、只剩标点。
    fn commit_with(&self, suffix: &str) {
        let word = self.current_word();
        // 学的是「拼上标点之前」的那个词。晚一步就会把「你好，」当成一个词记进去，
        // 而候选里永远不会有带标点的词，这条记录等于白存。
        crate::freq::bump(&word);
        let mut text = word;
        text.push_str(suffix);
        self.pending_op.borrow_mut().replace(EditOp::Commit(text));
        self.request_edit_session();
        self.clear_state();
    }

    /// 回车：把已经敲进去的编码原样上屏。
    ///
    /// 和空格的区别就在这儿——空格是「选第一个候选」，回车是「我就要这几个字母」。
    /// 快捷输入（`\rq`）例外：`\rq` 本身没有意义，退回第一个候选（当前日期）。
    fn commit_raw(&self) {
        let code = self.code.borrow().clone();
        let text = if code.starts_with(quick::PREFIX) {
            self.current_word()
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
        self.try_segment(&[]);
    }

    /// 尝试进入分词模式。成功返回 `true`（已 refresh + 更新组合串）。
    ///
    /// `prev` 是上一轮的分段选择：继续敲字重新分段时，如果某一段的码没变，
    /// 就沿用用户已经选过的词。否则「选完第一段再往下打」会把选择冲掉。
    fn try_segment(&self, prev: &[(String, String)]) -> bool {
        if self.assist_active.get() {
            return false;
        }
        let code = self.code.borrow().clone();
        if code.starts_with(quick::PREFIX) {
            return false; // 快捷输入不拆
        }
        if !self.candidates.borrow().is_empty() {
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
                let word = match prev.get(i) {
                    Some((pc, w)) if pc == c => w.clone(),
                    _ => FeiheTextService::lookup_any(c, 1)
                        .first()
                        .map(|s| s.to_string())
                        .unwrap_or_default(),
                };
                Segment {
                    code: c.clone(),
                    word,
                }
            })
            .collect();

        let n = segments.len();
        *self.segments.borrow_mut() = segments;
        // 光标留在原来的段上（首次进入是 0），越界就夹到末段
        self.seg_idx.set(self.seg_idx.get().min(n - 1));
        self.refresh();
        self.update_composition();
        crate::trace!("segment mode: {n} segments from '{code}'");
        true
    }

    /// 分词模式下确认当前段的第 `sel` 个候选（越界退第一个），光标后移一段；
    /// 已经是最后一段就整体上屏。
    fn confirm_segment(&self, sel: usize) {
        let n = self.segments.borrow().len();
        if n == 0 {
            return;
        }
        let idx = self.seg_idx.get().min(n - 1);
        let word = {
            let segs = self.segments.borrow();
            let list = FeiheTextService::lookup_any(&segs[idx].code, MAX_CANDIDATES);
            list.get(sel)
                .or_else(|| list.first())
                .map(|s| s.to_string())
        };
        let Some(word) = word else { return };
        self.segments.borrow_mut()[idx].word = word;

        if idx + 1 < n {
            self.seg_idx.set(idx + 1);
            self.refresh();
            self.update_composition();
        } else {
            self.commit_segments("");
        }
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
                crate::freq::bump(&s.word);
            }
        }
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

    /// 退出分词模式，并按普通退格删掉编码最后一个字符
    fn exit_segment_mode(&self) {
        self.segments.borrow_mut().clear();
        self.seg_idx.set(0);
        self.backspace();
    }

    /// 分词模式的按键处理。`←`/`→` 移段，`↑`/`↓` 和 `-`/`=` 翻页，数字选词，
    /// 空格确认，回车上屏，退格退出，编码键延长编码并重新分段，
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
            // 空格：确认当前段第一个候选
            0x20 => {
                self.confirm_segment(0);
                Ok(BOOL::from(true))
            }
            // 回车：整体上屏
            0x0D => {
                self.commit_segments("");
                Ok(BOOL::from(true))
            }
            // 1-9：选当前段第 n 个候选
            0x31..=0x39 => {
                self.confirm_segment((vk - 0x30) as usize - 1);
                Ok(BOOL::from(true))
            }
            // 退格：退出分词模式，回到普通编码态再删一个字符
            0x08 => {
                self.exit_segment_mode();
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
                let prev: Vec<(String, String)> = self
                    .segments
                    .borrow()
                    .iter()
                    .map(|s| (s.code.clone(), s.word.clone()))
                    .collect();

                self.code.borrow_mut().push(ch);
                self.segments.borrow_mut().clear();
                self.refresh();
                // 新编码可能整串就有候选了（或音节还没打完），那就退出分词模式
                if !self.try_segment(&prev) {
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
                    self.commit_segments(ch.encode_utf8(&mut buf));
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
            }
            self.refresh();
            self.update_composition();
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
        // 先克隆一份 Arc 快照，避免借用跨过 candidate::show（它会加锁、发窗口消息）
        let snapshot: Vec<Cow<'static, str>> = self.candidates.borrow().clone();
        if snapshot.is_empty() {
            candidate::hide();
            return;
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
            crate::trace!("caret: GetTextExt failed");
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
            crate::trace!("begin_composition: no caret range");
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
        crate::trace!(
            "begin_composition('{text}') done, caret=({},{})",
            self.caret_x.get(),
            self.caret_y.get()
        );
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
        crate::trace!("Activate: tid={tid}");

        // 焦点文档不一定已存在，容错处理（否则 Activate 失败会让服务被判为加载失败）
        let context = unsafe { tim.GetFocus() }
            .ok()
            .and_then(|doc| unsafe { doc.GetTop() }.ok());
        crate::trace!(
            "Activate: focus context = {}",
            if context.is_some() { "ok" } else { "none" }
        );

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
        crate::trace!(
            "Activate: scheme={} assist={} fullwidth={}",
            cfg.scheme.label(),
            cfg.assist_mode.label(),
            cfg.fullwidth
        );

        // 中英文开关。关着的时候 MSCTF 不会把按键交给 TIP，症状就是
        // 「切到飞鹤拼音后字母直接进应用、没有任何候选窗」。
        //
        // 只有在「我们认为当前是中文态」时才强制置 1：用户主动切到英文后
        // 不能再把它拉回来，否则他刚做的切换会被下一次激活抹掉。
        if insert::chinese() {
            open_keyboard(&tim, tid);
        } else {
            crate::trace!("Activate: keep english mode");
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
        crate::trace!("Activate: AdviseKeyEventSink -> {hr:?}");
        hr?;

        self.thread_mgr.borrow_mut().replace(tim);
        *self.context.borrow_mut() = context;
        self.tid.set(tid);
        // 软键盘 / 设置窗要靠这个 client id 才能 RequestEditSession 和改中英开关
        insert::set_client_tid(tid);
        statusbar::show();
        crate::trace!("Activate: done");
        Ok(())
    }

    fn Deactivate(&self) -> Result<()> {
        crate::trace!("Deactivate");
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

impl ITfTextInputProcessorEx_Impl for FeiheTextService_Impl {
    fn ActivateEx(&self, ptim: Ref<'_, ITfThreadMgr>, tid: u32, _dwflags: u32) -> Result<()> {
        self.Activate(ptim, tid)
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
            crate::trace!("TestKeyDown vk=0x{vk:02x}: modifier down -> false");
            return Ok(BOOL::from(false));
        }
        // 英文态：一个键都不吃，全部交给应用。
        // 这一步不能省——宿主不会替我们挡住按键（见 insert::CHINESE 的注释）。
        if !insert::chinese() {
            return Ok(BOOL::from(false));
        }
        if self.is_composing() {
            crate::trace!("TestKeyDown vk=0x{vk:02x}: composing -> true");
            return Ok(BOOL::from(true));
        }
        // 除了编码键，**全角标点键也必须认**。经典宿主（记事本、Word）里
        // OnTestKeyDown 返回 FALSE 就不会派发 OnKeyDown，我们连看都看不到这个键，
        // 全角标点开关等于失效。返回 TRUE 只是「让我看一眼」，OnKeyDown 里
        // 该拒绝的照样返回 FALSE，键最终还是会交给应用。
        let take = FeiheTextService::is_code_key(vk)
            || FeiheTextService::fullwidth_punct_for(vk).is_some();
        crate::trace!("TestKeyDown vk=0x{vk:02x} -> {take}");
        Ok(BOOL::from(take))
    }

    fn OnKeyDown(
        &self,
        pic: Ref<'_, ITfContext>,
        wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        let vk = (wparam.0 & 0xff) as u8;
        crate::trace!("KeyDown vk=0x{vk:02x} composing={}", self.is_composing());

        // 在应用里敲键就说明用户已经回到打字，软键盘可以收了
        softkbd::hide();

        if let Ok(context) = pic.ok() {
            self.context.borrow_mut().replace(context.clone());
        } else {
            crate::trace!("KeyDown: pic is null");
        }

        // ── Shift：单击切中英 ──────────────────────────────
        // 按下时什么也不做，键照常交给应用（大写字母要用它）；抬起时才判定。
        self.note_shift_combo(vk);
        if FeiheTextService::is_shift(vk) {
            return Ok(BOOL::from(false));
        }

        // 组合态下按修饰键（Ctrl / Alt / Win）：先把组合串提交，键原样交给应用。
        // 否则 Ctrl+A、Ctrl+C 会被当成字母吃进组合串。
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
                    crate::trace!(
                        "KeyDown: start composition code='{}' candidates={}",
                        self.code.borrow(),
                        self.candidates.borrow().len()
                    );
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
            // 左右：页内移动
            0x25 => {
                self.move_selection(-1);
                Ok(BOOL::from(true))
            }
            0x27 => {
                self.move_selection(1);
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
                if !next {
                    self.assist.borrow_mut().clear();
                }
                crate::trace!("KeyDown: assist -> {next} ({})", mode.label());
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
        let r = self.run_edit_session(ec);
        if let Err(e) = &r {
            crate::trace!("DoEditSession: ERR {e:?}");
        }
        r
    }
}

impl FeiheTextService_Impl {
    fn run_edit_session(&self, ec: u32) -> Result<()> {
        let Some(op) = self.pending_op.borrow_mut().take() else {
            return Ok(());
        };
        let Some(context) = self.context.borrow().as_ref().cloned() else {
            crate::trace!("DoEditSession: no context");
            self.composition.borrow_mut().take();
            return Ok(());
        };

        unsafe {
            match op {
                EditOp::Start => {
                    if self.begin_composition(&context, ec).is_err() {
                        crate::trace!("edit: Start -> no caret range");
                        self.clear_state();
                    }
                }
                EditOp::Update(text) => {
                    crate::trace!("edit: Update('{text}')");
                    // 先把引用 clone 出来，避免 RefCell 借用跨越 EndComposition 之类的重入
                    let comp = self.composition.borrow().as_ref().cloned();
                    if let Some(comp) = comp {
                        let r = comp.GetRange()?;
                        r.SetText(ec, 0, &to_utf16(&text))?;
                        r.Collapse(ec, TF_ANCHOR_END)?;
                        put_caret(&context, ec, &r);
                        self.remember_caret(&context, ec, &r);
                    } else {
                        crate::trace!("edit: Update -> no composition");
                    }
                }
                EditOp::Commit(text) => {
                    crate::trace!("edit: Commit('{text}')");
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
                        crate::trace!("edit: Commit -> no composition");
                    }
                    candidate::hide();
                }
                EditOp::Insert(text) => {
                    crate::trace!("edit: Insert('{text}')");
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
                    crate::trace!("edit: Cancel");
                    let comp = self.composition.borrow_mut().take();
                    if let Some(comp) = comp {
                        let r = comp.GetRange()?;
                        r.SetText(ec, 0, &[])?;
                        r.Collapse(ec, TF_ANCHOR_END)?;
                        put_caret(&context, ec, &r);
                        let _ = comp.EndComposition(ec);
                    }
                    candidate::hide();
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
        crate::trace!("compartment: no ITfCompartmentMgr");
        return;
    };
    let comp = match unsafe { mgr.GetCompartment(&GUID_COMPARTMENT_KEYBOARD_OPENCLOSE) } {
        Ok(c) => c,
        Err(e) => {
            crate::trace!("compartment: GetCompartment err {e:?}");
            return;
        }
    };
    match unsafe { comp.GetValue() } {
        Ok(v) => crate::trace!("compartment: openclose before = {}", describe_variant(&v)),
        Err(e) => crate::trace!("compartment: GetValue err {e:?}"),
    }

    let mut v = unsafe { VariantInit() };
    unsafe {
        // VARIANT_0.Anonymous 是 ManuallyDrop，需要显式解引用才能写字段
        let inner = &mut *v.Anonymous.Anonymous;
        inner.vt = VT_I4;
        inner.Anonymous.lVal = 1;
    }
    match unsafe { comp.SetValue(tid, &v) } {
        Ok(()) => crate::trace!("compartment: forced open (1)"),
        Err(e) => crate::trace!("compartment: SetValue err {e:?}"),
    }
    match unsafe { comp.GetValue() } {
        Ok(v) => crate::trace!("compartment: openclose after = {}", describe_variant(&v)),
        Err(e) => crate::trace!("compartment: re-read err {e:?}"),
    }
}

fn describe_variant(v: &VARIANT) -> String {
    unsafe {
        let vt = v.Anonymous.Anonymous.vt.0;
        // 3 = VT_I4
        if vt == 3 {
            format!("VT_I4({})", v.Anonymous.Anonymous.Anonymous.lVal)
        } else {
            format!("vt={vt}")
        }
    }
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

    /// 碰 `SCHEME` / `pinyin_mode` 的用例统一拿 dict 模块那把共享锁串行，
    /// 否则和 dict.rs 的测试并行跑会互相篡改（小鹤码在微软方案下查不到）。
    fn lock() -> std::sync::MutexGuard<'static, ()> {
        dict::tests::SCHEME_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner())
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
        assert!(FeiheTextService::code_allowed("`rq"));
        assert!(!FeiheTextService::code_allowed("`r1"));
        // 旧的 `\` 引导符已经让给顿号，不再是编码字符
        assert!(!FeiheTextService::code_allowed("\\rq"));
    }

    #[test]
    fn fullwidth_punct_only_for_unshifted_keys() {
        assert_eq!(FeiheTextService::fullwidth_punct(VK_OEM_COMMA.0 as u8), Some('，'));
        assert_eq!(FeiheTextService::fullwidth_punct(VK_OEM_PERIOD.0 as u8), Some('。'));
        // 顿号两个键都给：`\`（搜狗习惯）和 `/`（微软拼音习惯）
        assert_eq!(FeiheTextService::fullwidth_punct(VK_OEM_5.0 as u8), Some('、'));
        assert_eq!(FeiheTextService::fullwidth_punct(VK_OEM_2.0 as u8), Some('、'));
        // 反引号现在是快捷输入引导符，不再是间隔号
        assert_eq!(FeiheTextService::fullwidth_punct(VK_OEM_3.0 as u8), None);
        // 字母键不是标点
        assert_eq!(FeiheTextService::fullwidth_punct(0x41), None);
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
    fn backslash_and_backquote_keys_are_split() {
        // `\` 不再进编码：中文+全角标点时它输出「、」，Shift 组合交给应用出「|」
        assert!(!FeiheTextService::is_code_key(VK_OEM_5.0 as u8));
        assert_eq!(FeiheTextService::code_char(VK_OEM_5.0 as u8), None);
        // `` ` `` 接管快捷输入引导符
        assert!(FeiheTextService::is_code_key(VK_OEM_3.0 as u8));
        assert_eq!(
            FeiheTextService::code_char(VK_OEM_3.0 as u8),
            Some(crate::quick::PREFIX)
        );
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
        // 「仝」是踩过的坑：早期 gen_dict.py 按词频把单字截断到 3500 个，
        // 「仝」在 8105 字表里排 5402 名（词频 796，第 3500 名的下限是 11418），
        // 被直接截掉，用户翻页也翻不到。现在单字全收（7896 个），这里钉死。
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
