//! 全局状态：唯一的真相来源。
//!
//! 三个模块（窗口过程 / 文本存储 / UIElement sink）都跑在同一个 STA 线程上，
//! 但 COM 回调的入口是三个不同的对象，互相拿不到对方的字段。与其到处传
//! `Rc<RefCell<..>>`，不如丢进一个 `Mutex`——没有竞争，只是要一份共享黑板。
//!
//! 注意 `doc` 是**唯一**的文档内容：`Store::GetText` 从它读，窗口从它画，
//! 两处各存一份迟早会对不上（而 TSF 的文本一变，候选窗位置就跟着错）。

use std::sync::Mutex;

pub struct Shared {
    /// 文档内容（UTF-8，只为了画和转 UTF-16 方便）
    pub doc: String,
    /// 选区 / 光标，ACP 偏移
    pub sel: (i32, i32),

    /// 宿主拿到的候选（由 `ITfCandidateListUIElement` 读出来）
    pub candidates: Vec<String>,
    pub cand_sel: usize,
    pub cand_page: usize,
    /// 每页首项在整表里的下标（元素自己报的，不假设每页 9 个）
    pub pages: Vec<u32>,
    /// 当前是否有一份由宿主接管的候选在显示
    pub cand_active: bool,

    /// 统计：`BeginUIElement` / `UpdateUIElement` / `EndUIElement` 各被调了几次。
    /// 这是判断「UILess 到底通没通」最直接的证据——TIP 走自绘窗时这三个都是 0。
    pub begins: u32,
    pub updates: u32,
    pub ends: u32,

    /// 线程管理器实际记下的激活标志（`ITfThreadMgrEx::GetActiveFlags`）
    pub active_flags: u32,
    /// feihe.dll 是否已经加载进本进程
    pub tip_loaded: bool,
    /// TIP 有没有注册 `GUID_TFCAT_TIPCAP_UIELEMENTENABLED` 分类。
    /// 没有的话，UILess 模式下 MSCTF 根本不会加载它（不报错，输入法像没装一样）。
    pub category: bool,
    /// 一次性提示（加载失败之类）
    pub note: String,
}

pub static STATE: Mutex<Shared> = Mutex::new(Shared {
    doc: String::new(),
    sel: (0, 0),
    candidates: Vec::new(),
    cand_sel: 0,
    cand_page: 0,
    pages: Vec::new(),
    cand_active: false,
    begins: 0,
    updates: 0,
    ends: 0,
    active_flags: 0,
    tip_loaded: false,
    category: false,
    note: String::new(),
});

/// 拿状态，顺手把 poisons 当无事发生——这里没有会 panic 的临界区，
/// 真要 panic 了也是测试工具自己崩，没必再叠一层 panic。
pub fn lock() -> std::sync::MutexGuard<'static, Shared> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}
