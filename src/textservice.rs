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
    /// 把组合串左边能匹配的部分上屏，剩下的编码开一个新的组合串
    Split { commit: String, rest: String },
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

    /// 找出最长的、能出候选的前缀占多少个音节；找不到返回 0。
    ///
    /// 只按音节边界递减，所以双拼下切点一定落在偶数位、全拼下一定落在
    /// 合法音节之间——不会把 `zhuan` 切成 `zhu`+`an` 这种半截音节。
    fn longest_prefix_units(units: &[String], has: impl Fn(&str) -> bool) -> usize {
        if units.len() < 2 {
            return 0;
        }
        for k in (1..units.len()).rev() {
            if has(&units[..k].concat()) {
                return k;
            }
        }
        0
    }

    /// 是不是 Shift 键本身。宿主可能报 VK_SHIFT(0x10)，也可能报左右键(0xA0/0xA1)。
    fn is_shift(vk: u8) -> bool {
        vk == VK_SHIFT.0 as u8 || vk == 0xA0 || vk == 0xA1
    }

    /// 全角标点映射（只处理不带 Shift 的那一排键）。
    /// Shift 组合（`?` `:` `"` 等）交给应用，避免和我们自己算的字符对不上。
    fn fullwidth_punct(vk: u8) -> Option<char> {
        let ch = match vk {
            v if v == VK_OEM_COMMA.0 as u8 => '，',
            v if v == VK_OEM_PERIOD.0 as u8 => '。',
            v if v == VK_OEM_1.0 as u8 => '；',
            v if v == VK_OEM_2.0 as u8 => '、',
            v if v == VK_OEM_3.0 as u8 => '·',
            v if v == VK_OEM_4.0 as u8 => '【',
            v if v == VK_OEM_6.0 as u8 => '】',
            v if v == VK_OEM_7.0 as u8 => '’',
            v if v == VK_OEM_MINUS.0 as u8 => '－',
            v if v == VK_OEM_PLUS.0 as u8 => '＝',
            _ => return None,
        };
        Some(ch)
    }

    /// 该虚拟键是否属于当前方案的编码键。
    /// 双拼：小鹤只有字母；微软双拼的 `ing` 落在分号键上，所以还要放行 VK_OEM_1。
    /// 全拼：只有字母。
    /// 另外都接受手动分隔键（见 `is_separator_key`）和快捷输入的 `\`。
    fn is_code_key(vk: u8) -> bool {
        if vk.is_ascii_uppercase() {
            return true;
        }
        // 快捷输入的引导符 `\`
        if vk == VK_OEM_5.0 as u8 {
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
        if vk == VK_OEM_5.0 as u8 {
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
        // 快捷输入：`\` + 纯小写字母
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
                if i == cur {
                    s.push('[');
                    s.push_str(&seg.word);
                    s.push(']');
                } else {
                    s.push_str(&seg.word);
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
        let word = self.current_word();
        self.pending_op.borrow_mut().replace(EditOp::Commit(word));
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

    /// 音节已经打完、但整串一个候选都没有时，把左边能匹配的部分先上屏。
    ///
    /// 两条路，按优先级：
    /// 1. **分词模式**——整串能切成多段且每段都有候选时，挂起来让用户逐段挑
    ///    （`←`/`→` 移段、数字选词、空格确认、回车整体上屏）
    /// 2. **拆分上屏**——切不出多段时，把左边能匹配的部分直接上屏，剩下的开新一轮
    ///
    /// 这就是主流输入法的「边打边上屏」：打 `vhsjlisi`，词库里没有「张三李四」，
    /// 但不能就这么挂着一串没人可选的字母。
    ///
    /// 只在**音节完整**（双拼偶数长度 / 全拼能切出合法音节）且候选为空时触发，
    /// 否则用户每敲一个声母就被打断一次。
    fn maybe_split(&self) {
        if self.assist_active.get() {
            return;
        }
        let code = self.code.borrow().clone();
        if code.starts_with(quick::PREFIX) {
            return; // 快捷输入不拆
        }
        if !self.candidates.borrow().is_empty() {
            return; // 有候选，正常走
        }
        // 音节还没打完（双拼奇数长度 / 全拼切不出合法音节）→ 再等等
        let units = FeiheTextService::syllables(&code);
        if units.len() < 2 {
            return;
        }
        let has = |p: &str| !FeiheTextService::lookup_any(p, 1).is_empty();

        // 1) 先试分词模式（全拼和双拼都支持，音节边界由 syllables 决定）
        let codes = FeiheTextService::split_segments_units(&units, has);
        if codes.len() >= 2 {
            let segments: Vec<Segment> = codes
                .iter()
                .map(|c| Segment {
                    code: c.clone(),
                    word: FeiheTextService::lookup_any(c, 1)
                        .first()
                        .map(|s| s.to_string())
                        .unwrap_or_default(),
                })
                .collect();
            *self.segments.borrow_mut() = segments;
            self.seg_idx.set(0);
            self.refresh();
            self.update_composition();
            crate::trace!("segment mode: {} segments from '{code}'", codes.len());
            return;
        }

        // 2) 退回拆分上屏：最长可出候选的音节前缀
        let cut = FeiheTextService::longest_prefix_units(&units, has);
        if cut == 0 {
            // 一个音节都匹配不上（用户打了串不存在的东西），留给退格处理
            return;
        }
        let prefix: String = units[..cut].concat();
        let Some(word) = FeiheTextService::lookup_any(&prefix, 1)
            .first()
            .map(|s| s.to_string())
        else {
            return;
        };
        let rest: String = units[cut..].concat();
        crate::trace!("split: commit '{word}', rest '{rest}'");
        self.pending_op
            .borrow_mut()
            .replace(EditOp::Split { commit: word, rest });
        self.request_edit_session();
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
            self.commit_segments();
        }
    }

    /// 分词模式整体上屏：把所有段已选的词拼起来提交
    fn commit_segments(&self) {
        let text: String = self
            .segments
            .borrow()
            .iter()
            .map(|s| s.word.as_str())
            .collect();
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

    /// 分词模式的按键处理。`←`/`→` 移段，数字选词，空格确认，回车上屏，
    /// 退格退出，其余键整体上屏后原样交给应用。
    fn on_segment_key(&self, vk: u8) -> Result<BOOL> {
        let n = self.segments.borrow().len();
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
                self.commit_segments();
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
            // 别的键（含编码键）：先整体上屏，键再交给应用
            _ => {
                self.commit_segments();
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
    /// `EditOp::Start` 和 `EditOp::Split` 都要用——拆分时上一个组合刚结束，
    /// 必须在同一个 edit session 里把新的建起来，否则文档会留在
    /// 「字已上屏、光标悬空」的状态。
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
        let take = FeiheTextService::is_code_key(vk);
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
            if config::get().fullwidth && !FeiheTextService::shift_down() {
                if let Some(ch) = FeiheTextService::fullwidth_punct(vk) {
                    let mut buf = [0u8; 4];
                    self.insert_direct(ch.encode_utf8(&mut buf));
                    return Ok(BOOL::from(true));
                }
            }
            return Ok(BOOL::from(false));
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
            // 上下：翻页
            0x26 => {
                self.turn_page(-1);
                Ok(BOOL::from(true))
            }
            0x28 => {
                self.turn_page(1);
                Ok(BOOL::from(true))
            }
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
            // 其他键：先提交，再把键还给应用
            _ => {
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
                    let Ok(range) = caret_range(&context, ec) else {
                        return Ok(());
                    };
                    range.SetText(ec, 0, &to_utf16(&text))?;
                    range.Collapse(ec, TF_ANCHOR_END)?;
                    put_caret(&context, ec, &range);
                }
                EditOp::Split { commit, rest } => {
                    crate::trace!("edit: Split('{commit}', rest '{rest}')");
                    // 1) 结束当前组合，把左边这段换成实际的字
                    let comp = self.composition.borrow_mut().take();
                    if let Some(comp) = comp {
                        let r = comp.GetRange()?;
                        r.SetText(ec, 0, &to_utf16(&commit))?;
                        r.Collapse(ec, TF_ANCHOR_END)?;
                        put_caret(&context, ec, &r);
                        // EndComposition 会同步回调 OnCompositionTerminated → clear_state()，
                        // 我们的 code / candidates 会被清空。没关系，下面本来就要重设。
                        let _ = comp.EndComposition(ec);
                    }
                    // 2) 用剩下的编码立刻开一个新的组合串
                    *self.code.borrow_mut() = rest;
                    self.assist.borrow_mut().clear();
                    self.assist_active.set(false);
                    self.selected.set(0);
                    self.page.set(0);
                    self.refresh();
                    if self.begin_composition(&context, ec).is_err() {
                        self.clear_state();
                    }
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
    use std::sync::Mutex;

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
        // 快捷输入的 `\` + 字母是合法的
        assert!(FeiheTextService::code_allowed("\\rq"));
        assert!(!FeiheTextService::code_allowed("\\r1"));
    }

    #[test]
    fn fullwidth_punct_only_for_unshifted_keys() {
        assert_eq!(FeiheTextService::fullwidth_punct(VK_OEM_COMMA.0 as u8), Some('，'));
        assert_eq!(FeiheTextService::fullwidth_punct(VK_OEM_PERIOD.0 as u8), Some('。'));
        // 反斜杠现在是快捷输入引导符，不再映射顿号
        assert_eq!(FeiheTextService::fullwidth_punct(VK_OEM_5.0 as u8), None);
        // 字母键不是标点
        assert_eq!(FeiheTextService::fullwidth_punct(0x41), None);
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

    #[test]
    fn query_code_strips_separator() {
        assert_eq!(FeiheTextService::query_code("ni'hao").as_ref(), "nihao");
        assert_eq!(FeiheTextService::query_code("vhsj").as_ref(), "vhsj");
    }

    #[test]
    fn longest_prefix_units_respects_syllables() {
        let has = |p: &str| matches!(p, "vhsj" | "vh");
        let units = FeiheTextService::syllables_ds("vhsjzz");
        // vhsj(2 个音节) 能出候选 → 切在 2
        assert_eq!(FeiheTextService::longest_prefix_units(&units, has), 2);
        // 只有一个音节时不拆
        assert_eq!(
            FeiheTextService::longest_prefix_units(&FeiheTextService::syllables_ds("vh"), has),
            0
        );
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
        assert!(!FeiheTextService::can_insert_sep("\\rq"));
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

    /// 零声母词的可达性回归：`an` 在小鹤里是 `a` + 韵母键 `j` = `aj`，
    /// 所以「按钮」= `ajnq`，不是按全拼直觉敲的 `annq`。
    /// 这类词最容易「以为词库缺词」，其实是编码记错了，用测试把正确码钉住。
    #[test]
    fn zero_initial_words_are_reachable() {
        let _g = lock();
        dict::set_scheme(Scheme::Xiaohe);
        config::set_pinyin_mode(false);

        assert_eq!(FeiheTextService::syllables_ds("ajnq"), vec!["aj", "nq"]);

        // 硬编码一个，把「零声母 an → aj」这条用户可见规则钉死
        let list = dict::lookup("ajnq", 30);
        assert!(
            list.iter().any(|w| w.as_ref() == "按钮"),
            "ajnq 应出「按钮」，实际候选 {:?}",
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
    fn is_shift_recognizes_all_variants() {
        assert!(FeiheTextService::is_shift(0x10));
        assert!(FeiheTextService::is_shift(0xA0));
        assert!(FeiheTextService::is_shift(0xA1));
        assert!(!FeiheTextService::is_shift(0x41));
        assert!(!FeiheTextService::is_shift(0x0D));
    }
}
