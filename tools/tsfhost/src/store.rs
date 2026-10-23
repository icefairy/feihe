//! 最小 `ITextStoreACP`：一块单行、无属性的内存文本。
//!
//! TSF 里所有编辑动作最后都落到**宿主**实现的文本存储上。要让「TIP 能不能打字」
//! 这条路走通，只需实现下面这几件：
//!
//! - `RequestLock`：TSF 改文档前要锁，宿主必须在锁内同步回调 `OnLockGranted`；
//! - `GetText` / `SetText`：读写文本。TIP 开组合串、上屏、删码都走这里；
//! - `GetSelection` / `SetSelection`：光标；
//! - `GetTextExt` / `GetScreenExt` / `GetWnd`：光标和文档在屏幕上的位置，
//!   传统模式下自绘候选窗要靠它定位（UILess 模式下用不上，但 TIP 的两条路径共用代码）。
//!
//! 其余一律 `E_NOTIMPL`——这不是编辑器，只是测试床。

use std::cell::{Cell, RefCell};

use windows::core::{implement, Interface, Ref, Result, BOOL, GUID, HRESULT, PCWSTR, PWSTR};
use windows::Win32::Foundation::{E_INVALIDARG, E_NOTIMPL, HWND, POINT, RECT, SIZE, S_OK};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, GetDC, GetTextExtentPoint32W, InvalidateRect, ReleaseDC,
};
use windows::Win32::UI::TextServices::*;

use crate::state;

/// 文档正文的绘制起点（客户区坐标）。`GetTextExt` 按同一个原点点算屏幕位置。
pub const TEXT_LEFT: i32 = 16;
pub const TEXT_TOP: i32 = 64;
pub const LINE_H: i32 = 28;

/// `TS_E_NOLOCK`：还没 `AdviseSink` 就来要锁 / 读文本。
const TS_E_NOLOCK: HRESULT = HRESULT(0x8004_0200u32 as i32);

#[implement(ITextStoreACP)]
pub struct Store {
    hwnd: Cell<HWND>,
    sink: RefCell<Option<ITextStoreACPSink>>,
}

impl Store {
    pub fn new(hwnd: HWND) -> Self {
        Self {
            hwnd: Cell::new(hwnd),
            sink: RefCell::new(None),
        }
    }

    /// `[start, end)` 换成 `ins`，返回 TSF 要的 `TS_TEXTCHANGE`。
    fn replace(&self, start: i32, end: i32, ins: &[u16]) -> TS_TEXTCHANGE {
        let mut st = state::lock();
        let mut chars: Vec<u16> = st.doc.encode_utf16().collect();
        let len = chars.len() as i32;
        let s = start.clamp(0, len) as usize;
        let e = end.clamp(s as i32, len) as usize;

        chars.splice(s..e, ins.iter().copied());
        let new_end = s + ins.len();
        st.doc = String::from_utf16_lossy(&chars);
        st.sel = (new_end as i32, new_end as i32);
        drop(st);

        unsafe {
            let _ = InvalidateRect(Some(self.hwnd.get()), None, false);
        }
        TS_TEXTCHANGE {
            acpStart: s as i32,
            acpOldEnd: e as i32,
            acpNewEnd: new_end as i32,
        }
    }

    /// 文本前缀的屏幕矩形。单行布局，够 TIP 定位候选窗。
    fn text_rect(&self, from: i32, to: i32) -> RECT {
        let doc = state::lock().doc.clone();
        let u: Vec<u16> = doc.encode_utf16().collect();
        let cut = |acp: i32| -> usize { acp.clamp(0, u.len() as i32) as usize };

        let hwnd = self.hwnd.get();
        let w0 = unsafe { measure(hwnd, &u[..cut(from)]) };
        let w1 = unsafe { measure(hwnd, &u[..cut(to)]) };

        let mut origin = POINT {
            x: TEXT_LEFT,
            y: TEXT_TOP,
        };
        unsafe {
            let _ = ClientToScreen(hwnd, &mut origin);
        }
        RECT {
            left: origin.x + w0,
            top: origin.y,
            right: origin.x + w1,
            bottom: origin.y + LINE_H,
        }
    }
}

/// 用窗口 DC 量一段 UTF-16 的宽度（和绘制用同一个字体度量）。
unsafe fn measure(hwnd: HWND, s: &[u16]) -> i32 {
    if s.is_empty() {
        return 0;
    }
    let mut size = SIZE::default();
    let hdc = GetDC(Some(hwnd));
    if hdc.is_invalid() {
        return 0;
    }
    let _ = GetTextExtentPoint32W(hdc, s, &mut size);
    ReleaseDC(Some(hwnd), hdc);
    size.cx
}

impl ITextStoreACP_Impl for Store_Impl {
    fn AdviseSink(&self, _riid: *const GUID, punk: Ref<'_, windows::core::IUnknown>, _dwmask: u32) -> Result<()> {
        // TSF 只会用 IID_ITextStoreACPSink 来 advise；QI 不通就是调用方搞错了。
        let sink: ITextStoreACPSink = punk.ok()?.cast()?;
        *self.sink.borrow_mut() = Some(sink);
        Ok(())
    }

    fn UnadviseSink(&self, _punk: Ref<'_, windows::core::IUnknown>) -> Result<()> {
        self.sink.borrow_mut().take();
        Ok(())
    }

    /// 授予编辑锁。必须是**同步**的：在返回之前回调 `OnLockGranted`，
    /// TSF 的 `RequestEditSession(TF_ES_SYNC)` 就指望这一层同步完成。
    fn RequestLock(&self, dwlockflags: u32) -> Result<HRESULT> {
        let Some(sink) = self.sink.borrow().clone() else {
            return Ok(TS_E_NOLOCK);
        };
        match unsafe { sink.OnLockGranted(TEXT_STORE_LOCK_FLAGS(dwlockflags)) } {
            Ok(()) => Ok(S_OK),
            Err(e) => Ok(e.code()),
        }
    }

    fn GetStatus(&self) -> Result<TS_STATUS> {
        // 0 = 可读写、不是只读文档。动态标志（读写锁状态）我们懒得维护。
        Ok(TS_STATUS {
            dwDynamicFlags: 0,
            dwStaticFlags: 0,
        })
    }

    fn QueryInsert(&self, acpteststart: i32, _acptestend: i32, cch: u32, pacpresultstart: *mut i32, pacpresultend: *mut i32) -> Result<()> {
        unsafe {
            if !pacpresultstart.is_null() {
                *pacpresultstart = acpteststart;
            }
            if !pacpresultend.is_null() {
                *pacpresultend = acpteststart + cch as i32;
            }
        }
        Ok(())
    }

    fn GetSelection(&self, _ulindex: u32, _ulcount: u32, pselection: *mut TS_SELECTION_ACP, pcfetched: *mut u32) -> Result<()> {
        let sel = state::lock().sel;
        unsafe {
            if pselection.is_null() {
                return Err(E_INVALIDARG.into());
            }
            *pselection = TS_SELECTION_ACP {
                acpStart: sel.0,
                acpEnd: sel.1,
                style: TS_SELECTIONSTYLE {
                    ase: TS_AE_END,
                    fInterimChar: BOOL(0),
                },
            };
            if !pcfetched.is_null() {
                *pcfetched = 1;
            }
        }
        Ok(())
    }

    fn SetSelection(&self, ulcount: u32, pselection: *const TS_SELECTION_ACP) -> Result<()> {
        if ulcount == 0 || pselection.is_null() {
            return Ok(());
        }
        let sel = unsafe { *pselection };
        state::lock().sel = (sel.acpStart, sel.acpEnd);
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd.get()), None, false);
        }
        Ok(())
    }

    fn GetText(
        &self,
        acpstart: i32,
        acpend: i32,
        pchplain: PWSTR,
        cchplainreq: u32,
        pcchplainret: *mut u32,
        prgruninfo: *mut TS_RUNINFO,
        cruninforeq: u32,
        pcruninforet: *mut u32,
        pacpnext: *mut i32,
    ) -> Result<()> {
        let text: Vec<u16> = state::lock().doc.encode_utf16().collect();
        let len = text.len() as i32;
        let start = acpstart.clamp(0, len);
        let end = if acpend < 0 { len } else { acpend.clamp(0, len) }.max(start);
        let avail = (end - start) as usize;

        let mut copied = 0usize;
        unsafe {
            if cchplainreq > 0 && !pchplain.0.is_null() {
                copied = avail.min(cchplainreq as usize);
                core::ptr::copy_nonoverlapping(text[start as usize..].as_ptr(), pchplain.0, copied);
                if !pcchplainret.is_null() {
                    *pcchplainret = copied as u32;
                }
                if !pacpnext.is_null() {
                    *pacpnext = start + copied as i32;
                }
            }
            // 只问 run info（`cchPlainReq == 0`）时才需要给：我们全文都是普通文本。
            if cruninforeq > 0 && !prgruninfo.is_null() && copied == 0 {
                *prgruninfo = TS_RUNINFO {
                    uCount: avail as u32,
                    r#type: TsRunType(0), // TS_RUNINFO_TEXT
                };
                if !pcruninforet.is_null() {
                    *pcruninforet = 1;
                }
                if !pacpnext.is_null() {
                    *pacpnext = end;
                }
            }
        }
        Ok(())
    }

    fn SetText(&self, _dwflags: u32, acpstart: i32, acpend: i32, pchtext: &PCWSTR, cch: u32) -> Result<TS_TEXTCHANGE> {
        let ins: Vec<u16> = if cch == 0 || pchtext.0.is_null() {
            Vec::new()
        } else {
            unsafe { core::slice::from_raw_parts(pchtext.0, cch as usize) }.to_vec()
        };
        Ok(self.replace(acpstart, acpend, &ins))
    }

    fn GetFormattedText(&self, _acpstart: i32, _acpend: i32) -> Result<windows::Win32::System::Com::IDataObject> {
        Err(E_NOTIMPL.into())
    }

    fn GetEmbedded(&self, _acppos: i32, _rguidservice: *const GUID, _riid: *const GUID) -> Result<windows::core::IUnknown> {
        Err(E_NOTIMPL.into())
    }

    fn QueryInsertEmbedded(&self, _pguidservice: *const GUID, _pformatetc: *const windows::Win32::System::Com::FORMATETC) -> Result<BOOL> {
        Ok(BOOL(0))
    }

    fn InsertEmbedded(&self, _dwflags: u32, _acpstart: i32, _acpend: i32, _pdataobject: Ref<'_, windows::Win32::System::Com::IDataObject>) -> Result<TS_TEXTCHANGE> {
        Err(E_NOTIMPL.into())
    }

    fn InsertTextAtSelection(&self, _dwflags: u32, pchtext: &PCWSTR, cch: u32, pacpstart: *mut i32, pacpend: *mut i32, pchange: *mut TS_TEXTCHANGE) -> Result<()> {
        let ins: Vec<u16> = if cch == 0 || pchtext.0.is_null() {
            Vec::new()
        } else {
            unsafe { core::slice::from_raw_parts(pchtext.0, cch as usize) }.to_vec()
        };
        let sel = state::lock().sel;
        let change = self.replace(sel.0, sel.1, &ins);
        unsafe {
            if !pacpstart.is_null() {
                *pacpstart = change.acpStart;
            }
            if !pacpend.is_null() {
                *pacpend = change.acpNewEnd;
            }
            if !pchange.is_null() {
                *pchange = change;
            }
        }
        Ok(())
    }

    fn InsertEmbeddedAtSelection(&self, _dwflags: u32, _pdataobject: Ref<'_, windows::Win32::System::Com::IDataObject>, _pacpstart: *mut i32, _pacpend: *mut i32, _pchange: *mut TS_TEXTCHANGE) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn RequestSupportedAttrs(&self, _dwflags: u32, _cfilterattrs: u32, _pafilterattrs: *const GUID) -> Result<()> {
        Ok(())
    }

    fn RequestAttrsAtPosition(&self, _acppos: i32, _cfilterattrs: u32, _pafilterattrs: *const GUID, _dwflags: u32) -> Result<()> {
        Ok(())
    }

    fn RequestAttrsTransitioningAtPosition(&self, _acppos: i32, _cfilterattrs: u32, _pafilterattrs: *const GUID, _dwflags: u32) -> Result<()> {
        Ok(())
    }

    fn FindNextAttrTransition(&self, _acpstart: i32, _acphalt: i32, _cfilterattrs: u32, _pafilterattrs: *const GUID, _dwflags: u32, _pacpnext: *mut i32, _pffound: *mut BOOL, _plfoundoffset: *mut i32) -> Result<()> {
        Ok(())
    }

    fn RetrieveRequestedAttrs(&self, _ulcount: u32, _paattrvals: *mut TS_ATTRVAL, _pcfetched: *mut u32) -> Result<()> {
        Ok(())
    }

    fn GetEndACP(&self) -> Result<i32> {
        Ok(state::lock().doc.encode_utf16().count() as i32)
    }

    fn GetActiveView(&self) -> Result<u32> {
        Ok(0)
    }

    fn GetACPFromPoint(&self, _vcview: u32, _ptscreen: *const POINT, _dwflags: u32) -> Result<i32> {
        Ok(0)
    }

    fn GetTextExt(&self, _vcview: u32, acpstart: i32, acpend: i32, prc: *mut RECT, pfclipped: *mut BOOL) -> Result<()> {
        unsafe {
            if !prc.is_null() {
                *prc = self.text_rect(acpstart, acpend);
            }
            if !pfclipped.is_null() {
                *pfclipped = BOOL(0);
            }
        }
        Ok(())
    }

    fn GetScreenExt(&self, _vcview: u32) -> Result<RECT> {
        let hwnd = self.hwnd.get();
        let mut tl = POINT { x: 0, y: 0 };
        let mut br = POINT {
            x: crate::client_w(hwnd),
            y: crate::client_h(hwnd),
        };
        unsafe {
            let _ = ClientToScreen(hwnd, &mut tl);
            let _ = ClientToScreen(hwnd, &mut br);
        }
        Ok(RECT {
            left: tl.x,
            top: tl.y,
            right: br.x,
            bottom: br.y,
        })
    }

    fn GetWnd(&self, _vcview: u32) -> Result<HWND> {
        Ok(self.hwnd.get())
    }
}
