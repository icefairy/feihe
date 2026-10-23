//! TSF **UILess**：把候选交给宿主自己画
//!
//! ## 为什么需要这个模块
//!
//! 独占全屏的游戏会绕开 DWM，输入法自己开的窗口（候选窗、状态浮窗）画了也不上屏。
//! 微软为此在 TSF 里留了 UILess 通道：宿主调
//! `ITfThreadMgrEx::ActivateEx(TF_AE_UIELEMENTENABLEDONLY)`，TSF 再把同一个标志
//! 透传给 TIP 的 `ITfTextInputProcessorEx::ActivateEx`。之后：
//!
//! 1. TIP 想显示候选前必须先 `ITfUIElementMgr::BeginUIElement` 请示；
//! 2. 宿主在 `ITfUIElementSink::BeginUIElement` 里返回 `FALSE` = 「你别画，我来」；
//! 3. TIP 于是不建任何窗口，只在数据变化时 `UpdateUIElement(id)`；宿主自己 QI
//!    `ITfCandidateListUIElement` 取候选，用自己的字体和纹理画在游戏界面里。
//!
//! 这就是《魔兽世界》《原神》这类游戏里能打中文的原因——不是输入法有黑科技，
//! 是**游戏在画候选框**。反过来说，输入法这边不实现这套接口，那些游戏里就等于没有候选。
//!
//! ## 和 `config::no_popup` 的区别
//!
//! `no_popup` 是**用户**选的：别弹窗，候选写进组合串。这里这条是**宿主**要求的：
//! 候选由宿主渲染，体验比塞进组合串好（看得见列表、能翻页、能点选）。两者互斥——
//! 宿主接管时不再往组合串里塞一份，否则屏幕上会出现两份候选。
//!
//! ## 两个坑
//!
//! - `UpdateUIElement` 会**同步**回调进本对象的 `GetString`，所以改数据必须先放锁，
//!   否则自己等自己。
//! - 元素只在 `[BeginUIElement, EndUIElement)` 区间内有效，每次开始新组合都重新建
//!   一个对象；复用同一个在部分宿主上会读到上一轮的旧数据。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use windows::core::*;
use windows::Win32::Foundation::{E_FAIL, E_INVALIDARG};
use windows::Win32::UI::TextServices::*;
use windows_core::implement;

use crate::candidate::PAGE_SIZE;

/// 候选列表元素的标识。宿主靠它区分「还是那个元素，只是数据变了」。
const GUID_FEIHE_CANDIDATE: GUID = GUID::from_u128(0x6f2d1c84_5b3e_4a97_9d21_8c4e7b03a5f6);

/// `GetDescription` 返回的描述串（调试器/无障碍工具会读）
const DESCRIPTION: &str = "飞鹤拼音候选";

/// `ITfThreadMgrEx::ActivateEx` 的标志：宿主是 UI-less 线程，TIP 不许自己弹 UI。
///
/// windows-rs 没导出这个常量（`TF_AE_NONE` 那个名字被 `TfActiveSelEnd` 占了），
/// 值取自 tsf.h。
pub const TF_AE_UIELEMENTENABLEDONLY: u32 = 0x0000_0001;

/// 同一件事，但这是**宿主**传给 `ActivateEx` 的那份（msctf.h 的 `TfTmaeFlags`）。
///
/// 两套枚举长得像、值不一样：宿主传 4，MSCTF 转成上面的 1 再给 TIP。
/// 混用不会报错，只会让 UILess 静默失效——所以两个都留在这里当对照。
pub const TF_TMAE_UIELEMENTENABLEDONLY: u32 = 0x0000_0004;

/// 候选数据。本对象和 `Session` 共用一份，靠 `Rc<RefCell<_>>` 传递。
struct Data {
    /// 词库原文（简体），显示时再转简繁——和自绘候选窗一个规矩
    items: Vec<String>,
    /// 选中项在**整个列表**里的索引（不是页内索引）
    selected: usize,
    page: usize,
}

/// 交给宿主的候选列表元素。
///
/// `ITfCandidateListUIElement` 继承 `ITfUIElement`，两个 `_Impl` 要分开实现
/// （和 `FeiheTextService` 处理 `ITfTextInputProcessor(Ex)` 的关系一样）。
#[implement(ITfUIElement, ITfCandidateListUIElement)]
struct CandidateElement {
    data: Rc<RefCell<Data>>,
    doc: RefCell<Option<ITfDocumentMgr>>,
    /// 宿主有没有把它画出来。宿主自己决定显隐，我们只记录。
    shown: Cell<bool>,
}

impl ITfUIElement_Impl for CandidateElement_Impl {
    fn GetDescription(&self) -> Result<BSTR> {
        Ok(BSTR::from(DESCRIPTION))
    }

    fn GetGUID(&self) -> Result<GUID> {
        Ok(GUID_FEIHE_CANDIDATE)
    }

    fn Show(&self, bshow: BOOL) -> Result<()> {
        self.shown.set(bshow.as_bool());
        Ok(())
    }

    fn IsShown(&self) -> Result<BOOL> {
        Ok(BOOL::from(self.shown.get()))
    }
}

impl ITfCandidateListUIElement_Impl for CandidateElement_Impl {
    /// 报告自上次 `UpdateUIElement` 以来哪些属性变了。
    ///
    /// 老实算差集要记一份上次的快照，收益为零——宿主拿到全量标志只会多读几次
    /// `GetString`，而且候选页本来就是整页换掉的。
    fn GetUpdatedFlags(&self) -> Result<u32> {
        Ok(TF_CLUIE_DOCUMENTMGR
            | TF_CLUIE_COUNT
            | TF_CLUIE_SELECTION
            | TF_CLUIE_STRING
            | TF_CLUIE_PAGEINDEX
            | TF_CLUIE_CURRENTPAGE)
    }

    fn GetDocumentMgr(&self) -> Result<ITfDocumentMgr> {
        self.doc
            .borrow()
            .clone()
            .ok_or_else(|| Error::from(E_FAIL))
    }

    fn GetCount(&self) -> Result<u32> {
        Ok(self.data.borrow().items.len() as u32)
    }

    fn GetSelection(&self) -> Result<u32> {
        Ok(self.data.borrow().selected as u32)
    }

    /// 返回**候选词本身**，不要带 `1.` 这种序号——序号由宿主自己排。
    fn GetString(&self, uindex: u32) -> Result<BSTR> {
        let d = self.data.borrow();
        match d.items.get(uindex as usize) {
            Some(w) => {
                let text = crate::zhconv::apply(w);
                Ok(BSTR::from(text.as_ref()))
            }
            None => Err(Error::from(E_INVALIDARG)),
        }
    }

    /// 每页第一项在整表里的索引。`pindex` 可以是 NULL（宿主只问页数）。
    fn GetPageIndex(&self, pindex: *mut u32, size: u32, pupagecnt: *mut u32) -> Result<()> {
        let pages = page_starts(self.data.borrow().items.len());
        if !pupagecnt.is_null() {
            unsafe { *pupagecnt = pages.len() as u32 };
        }
        if !pindex.is_null() && size > 0 {
            for (i, start) in pages.iter().take(size as usize).enumerate() {
                unsafe { *pindex.add(i) = *start };
            }
        }
        Ok(())
    }

    /// 宿主告诉我们它自己的分页方式。我们固定每页 9 个（数字键就是 9 个），
    /// 记下来也没用，直接接受。
    fn SetPageIndex(&self, _pindex: *const u32, _upagecnt: u32) -> Result<()> {
        Ok(())
    }

    fn GetCurrentPage(&self) -> Result<u32> {
        Ok(self.data.borrow().page as u32)
    }
}

/// 每页第一项在整表里的索引。
///
/// 抽成纯函数才能单测——`CandidateElement_Impl` 在测试进程里构造不出来。
fn page_starts(count: usize) -> Vec<u32> {
    (0..count.div_ceil(PAGE_SIZE))
        .map(|i| (i * PAGE_SIZE) as u32)
        .collect()
}

/// 一次激活期间的 UILess 会话：负责请示、推送、收尾。
pub struct Session {
    mgr: ITfUIElementMgr,
    doc: Option<ITfDocumentMgr>,
    data: Rc<RefCell<Data>>,
    /// 已登记的 UI 元素 id；`None` = 当前没有在显示的元素
    id: Cell<Option<u32>>,
}

impl Session {
    pub fn new(mgr: ITfUIElementMgr, doc: Option<ITfDocumentMgr>) -> Self {
        Self {
            mgr,
            doc,
            data: Rc::new(RefCell::new(Data {
                items: Vec::new(),
                selected: 0,
                page: 0,
            })),
            id: Cell::new(None),
        }
    }

    /// 把候选推给宿主。
    ///
    /// 返回 `true` = 宿主接管绘制（调用方**不要**再开自己的窗口）；
    /// 返回 `false` = 宿主不接（`pbShow == TRUE` 或接口不可用），调用方自己画。
    pub fn update(&self, items: Vec<String>, selected: usize, page: usize) -> bool {
        // 先改数据、立刻放锁：下面的 COM 调用会同步回调进 GetString
        {
            let mut d = self.data.borrow_mut();
            d.items = items;
            d.selected = selected;
            d.page = page;
        }

        if let Some(id) = self.id.get() {
            let _ = unsafe { self.mgr.UpdateUIElement(id) };
            return true;
        }

        let el: ITfCandidateListUIElement = CandidateElement {
            data: Rc::clone(&self.data),
            doc: RefCell::new(self.doc.clone()),
            shown: Cell::new(false),
        }
        .into();
        let Ok(base) = el.cast::<ITfUIElement>() else {
            return false;
        };

        let mut show = BOOL(1);
        let mut id = 0u32;
        if unsafe { self.mgr.BeginUIElement(&base, &mut show, &mut id) }.is_err() {
            return false;
        }
        if show.as_bool() {
            // 宿主说「你自己画」——这条元素用不上了，注销掉，调用方走自绘窗
            let _ = unsafe { self.mgr.EndUIElement(id) };
            return false;
        }

        self.id.set(Some(id));
        // 首次数据要主动推一次；宿主是在这次调用里才开始画第一帧的
        let _ = unsafe { self.mgr.UpdateUIElement(id) };
        true
    }

    /// 结束当前元素（提交/取消/失焦都要调，否则宿主的候选框会一直挂着）
    pub fn end(&self) {
        if let Some(id) = self.id.take() {
            let _ = unsafe { self.mgr.EndUIElement(id) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 标志位别写错——写错了整条 UILess 通道就永远不开，
    /// 而且症状是「游戏里没候选」，很难看出来是常量的问题。
    #[test]
    fn activate_flag_matches_tsf_header() {
        assert_eq!(TF_AE_UIELEMENTENABLEDONLY, 0x1);
    }

    /// 分页边界：宿主靠这个数组翻页，差一个就是「最后一页少一个候选」。
    #[test]
    fn page_starts_are_page_aligned() {
        assert!(page_starts(0).is_empty(), "没候选就没有页");
        assert_eq!(page_starts(1), vec![0]);
        // 正好一页 / 一页零一个，是最容易写错的两种边界
        assert_eq!(page_starts(PAGE_SIZE), vec![0]);
        assert_eq!(page_starts(PAGE_SIZE + 1), vec![0, PAGE_SIZE as u32]);
        assert_eq!(page_starts(PAGE_SIZE * 2), vec![0, PAGE_SIZE as u32]);
        assert_eq!(
            page_starts(PAGE_SIZE * 2 + 1),
            vec![0, PAGE_SIZE as u32, (PAGE_SIZE * 2) as u32]
        );
    }
}
