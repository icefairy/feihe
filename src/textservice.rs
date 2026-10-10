// 飞鹤输入法 · TSF 文本服务核心
// M1 冒烟：激活 → 收键 → 同步 edit session 上屏（打全链路）
// M2 接入点：OnKeyDown（双拼/候选引擎）
use crate::clsid::LANGID_CHS;
use windows::core::*;
use windows_core::{implement, IUnknownImpl};
use windows::Win32::Foundation::*;
use windows::Win32::UI::TextServices::*;

use std::cell::{Cell, RefCell};

#[implement(
    ITfTextInputProcessorEx,
    ITfTextInputProcessor,
    ITfKeyEventSink,
    ITfEditSession
)]
pub struct FeiheTextService {
    tid: Cell<u32>,
    thread_mgr: RefCell<Option<ITfThreadMgr>>,
    context: RefCell<Option<ITfContext>>,
    /// M1 冒烟：待上屏文本
    pending_text: RefCell<Option<String>>,
}

impl FeiheTextService {
    pub fn new() -> Self {
        Self {
            tid: Cell::new(0),
            thread_mgr: RefCell::new(None),
            context: RefCell::new(None),
            pending_text: RefCell::new(None),
        }
    }
}

// ─────────────────────────────────────────────────────────────
// 生命周期：Activate / Deactivate
// ─────────────────────────────────────────────────────────────

impl ITfTextInputProcessor_Impl for FeiheTextService_Impl {
    fn Activate(&self, ptim: Ref<'_, ITfThreadMgr>, tid: u32) -> Result<()> {
        let tim: ITfThreadMgr = ptim.ok()?.clone();
        let doc: ITfDocumentMgr = unsafe { tim.GetFocus() }?;
        let context: ITfContext = unsafe { doc.GetTop() }?;

        // 按键 sink：CTF 把键盘事件转给 AdviseKeyEventSink 的 sink
        let keystroke_mgr: ITfKeystrokeMgr = tim.cast()?;
        let key_sink: ITfKeyEventSink = self.to_interface();
        unsafe { keystroke_mgr.AdviseKeyEventSink(tid, &key_sink, false)? };

        self.thread_mgr.borrow_mut().replace(tim);
        self.context.borrow_mut().replace(context);
        self.tid.set(tid);
        Ok(())
    }

    fn Deactivate(&self) -> Result<()> {
        if let Some(tim) = self.thread_mgr.borrow().as_ref() {
            if let Ok(mgr) = tim.cast::<ITfKeystrokeMgr>() {
                unsafe {
                    let _ = mgr.UnadviseKeyEventSink(self.tid.get());
                }
            }
        }
        self.thread_mgr.borrow_mut().take();
        self.context.borrow_mut().take();
        Ok(())
    }
}

impl ITfTextInputProcessorEx_Impl for FeiheTextService_Impl {
    fn ActivateEx(
        &self,
        ptim: Ref<'_, ITfThreadMgr>,
        tid: u32,
        _dwflags: u32,
    ) -> Result<()> {
        self.Activate(ptim, tid)
    }
}

// ─────────────────────────────────────────────────────────────
// 按键处理：M1 全部吃掉 + 上屏冒烟字符；M2 在这里进双拼引擎
// ─────────────────────────────────────────────────────────────

impl ITfKeyEventSink_Impl for FeiheTextService_Impl {
    fn OnSetFocus(&self, _fforeground: BOOL) -> Result<()> {
        Ok(())
    }

    fn OnTestKeyDown(
        &self,
        _pic: Ref<'_, ITfContext>,
        _wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        // M1：一律 eat（冒烟）；M2 起 composing 状态才 eat
        Ok(BOOL::from(true))
    }

    fn OnKeyDown(
        &self,
        pic: Ref<'_, ITfContext>,
        wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        let vk = (wparam.0 & 0xff) as u8;
        // M1 冒烟：任意字母键 → 上屏 "飞<字母>"，证明 TSF 全链路通
        if vk.is_ascii_graphic() {
            if let Ok(context) = pic.ok() {
                self.context.borrow_mut().replace(context.clone());
            }
            self.pending_text
                .borrow_mut()
                .replace(format!("飞{}", vk as char));
            self.request_edit_session();
        }
        Ok(BOOL::from(true))
    }

    fn OnTestKeyUp(
        &self,
        _pic: Ref<'_, ITfContext>,
        _wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        Ok(BOOL::from(true))
    }

    fn OnKeyUp(
        &self,
        _pic: Ref<'_, ITfContext>,
        _wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        Ok(BOOL::from(true))
    }

    fn OnPreservedKey(
        &self,
        _pic: Ref<'_, ITfContext>,
        _rguid: *const GUID,
    ) -> Result<BOOL> {
        Ok(BOOL::from(false))
    }
}

// ─────────────────────────────────────────────────────────────
// Edit session：上屏（同步）
// ─────────────────────────────────────────────────────────────

impl ITfEditSession_Impl for FeiheTextService_Impl {
    fn DoEditSession(&self, ec: u32) -> Result<()> {
        let Some(text) = self.pending_text.borrow_mut().take() else {
            return Ok(());
        };
        let Some(context) = self.context.borrow().as_ref().cloned() else {
            return Ok(());
        };
        unsafe {
            // 取当前 selection，插入文本
            let mut sel = [TF_SELECTION::default()];
            let mut fetched: u32 = 0;
            context.GetSelection(
                ec,
                TF_DEFAULT_SELECTION,
                &mut sel,
                &mut fetched,
            )?;
            if fetched == 0 {
                return Ok(());
            }
            let Some(range) = sel[0].range.take() else {
                return Ok(());
            };
            range.SetText(ec, 0, &to_utf16(&text))?;
            // 光标移到插入文本末尾
            range.Collapse(ec, TF_ANCHOR_END)?;
            sel[0].range.replace(range);
            context.SetSelection(ec, &sel)?;
        }
        Ok(())
    }
}

impl FeiheTextService_Impl {
    fn request_edit_session(&self) {
        let Some(context) = self.context.borrow().as_ref().cloned() else {
            return;
        };
        let session: ITfEditSession = self.to_interface();
        unsafe {
            let _ = context.RequestEditSession(
                self.tid.get(),
                &session,
                TF_ES_SYNC | TF_ES_READWRITE,
            );
        }
    }
}

fn to_utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

// LANGID 常量占位引用（注册表注册用）
#[allow(dead_code)]
fn _lang_check() -> u16 {
    LANGID_CHS
}
