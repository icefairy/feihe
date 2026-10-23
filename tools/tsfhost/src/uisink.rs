//! 宿主侧的 `ITfUIElementSink`：**这就是 UILess 的宿主那一半**。
//!
//! TIP 想画候选时会先 `ITfUIElementMgr::BeginUIElement` 请示，宿主在这里做选择：
//!
//! - `*pbshow = TRUE`  → 「你自己画」（TIP 走自绘窗，全屏游戏里等于看不见）
//! - `*pbshow = FALSE` → 「别画，我来」  ← 本程序的行为，等价于《原神》这类游戏
//!
//! 之后 TIP 只负责 `UpdateUIElement(id)` 推数据，宿主用 `GetUIElement(id)` 把元素
//! 取回来，QI 成 `ITfCandidateListUIElement` 逐项读 `GetString` 自己渲染。
//!
//! 换句话说：**只要 BeginUIElement 一次都没被调用，就说明 UILess 通道没通**，
//! 而 TIP 那边会退回自绘窗——这时屏幕上会出现飞鹤自己的候选框，反而是个显眼的证据。

use std::cell::Cell;

use windows::core::{implement, Interface, Result, BOOL};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::TextServices::*;
use windows::Win32::Graphics::Gdi::InvalidateRect;

use crate::state;

#[implement(ITfUIElementSink)]
pub struct UISink {
    mgr: ITfUIElementMgr,
    hwnd: Cell<HWND>,
}

impl UISink {
    pub fn new(mgr: ITfUIElementMgr, hwnd: HWND) -> Self {
        Self {
            mgr,
            hwnd: Cell::new(hwnd),
        }
    }

    fn repaint(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd.get()), None, false);
        }
    }

    /// 从元素 id 把候选整表读出来（数量、文本、当前页、选中项）。
    fn reload(&self, id: u32) {
        let Ok(el) = (unsafe { self.mgr.GetUIElement(id) }) else {
            return;
        };
        let Ok(list) = el.cast::<ITfCandidateListUIElement>() else {
            return;
        };

        let count = unsafe { list.GetCount() }.unwrap_or(0);
        let mut items = Vec::with_capacity(count as usize);
        for i in 0..count {
            if let Ok(s) = unsafe { list.GetString(i) } {
                items.push(s.to_string());
            }
        }

        let sel = unsafe { list.GetSelection() }.unwrap_or(0) as usize;
        let page = unsafe { list.GetCurrentPage() }.unwrap_or(0) as usize;
        let pages = page_starts(&list);

        eprintln!(
            "[ui] id={id} count={} sel={sel} page={page} pages={pages:?} :: {}",
            items.len(),
            items.join(" ")
        );

        let mut st = state::lock();
        st.candidates = items;
        st.cand_sel = sel;
        st.cand_page = page;
        st.pages = pages;
        st.cand_active = true;
    }
}

/// 问元素要分页表：第一次只问页数，第二次把每页首项的下标取回来。
fn page_starts(list: &ITfCandidateListUIElement) -> Vec<u32> {
    let mut count = 0u32;
    unsafe {
        // 传空切片 = 只问「一共几页」
        let _ = list.GetPageIndex(&mut [], &mut count);
    }
    if count == 0 {
        return vec![0];
    }
    let mut v = vec![0u32; count as usize];
    unsafe {
        let _ = list.GetPageIndex(&mut v, &mut count);
    }
    v
}

impl ITfUIElementSink_Impl for UISink_Impl {
    /// 关键的一行：`*pbshow = FALSE` 告诉 TIP「你别建窗口，候选我来画」。
    ///
    /// 这个回调是 TIP 的 `BeginUIElement` **同步**进来的，所以这里不能做重活——
    /// 只登记 + 立刻读一遍数据 + 让窗口重画。
    fn BeginUIElement(&self, dwuielementid: u32, pbshow: *mut BOOL) -> Result<()> {
        unsafe {
            if !pbshow.is_null() {
                *pbshow = BOOL(0);
            }
        }
        {
            let mut st = state::lock();
            st.begins += 1;
        }
        eprintln!("[ui] BeginUIElement id={dwuielementid} -> 宿主自绘（返回 FALSE）");
        self.reload(dwuielementid);
        self.repaint();
        Ok(())
    }

    fn UpdateUIElement(&self, dwuielementid: u32) -> Result<()> {
        {
            let mut st = state::lock();
            st.updates += 1;
        }
        self.reload(dwuielementid);
        self.repaint();
        Ok(())
    }

    fn EndUIElement(&self, _dwuielementid: u32) -> Result<()> {
        {
            let mut st = state::lock();
            st.ends += 1;
            st.cand_active = false;
            st.candidates.clear();
            st.pages.clear();
        }
        self.repaint();
        Ok(())
    }
}
