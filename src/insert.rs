//! 从任意窗口往当前文档写文本 / 读写中英开关
//!
//! 软键盘和设置窗都运行在宿主进程的 UI 线程上（和 TIP 同线程），所以可以直接
//! `TF_GetThreadMgr()` 拿到本线程的 TSF 管理器，再用同步 edit session 写文档。
//! 不需要把 TIP 的接口指针传出来，也不用跨线程 marshaling。
//!
//! 唯一需要外部告诉这里的信息是 **client id**（`ITfTextInputProcessor::Activate`
//! 的第二个参数）——`RequestEditSession` 和 `ITfCompartment::SetValue` 都要它。
//! TIP 激活时通过 `set_client_tid` 登记一次。

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use windows::core::*;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::System::Variant::{VariantInit, VT_I4};
use windows::Win32::UI::TextServices::*;
use windows_core::implement;

static CLIENT_TID: AtomicU32 = AtomicU32::new(0);

/// TIP 激活时登记 client id
pub fn set_client_tid(tid: u32) {
    CLIENT_TID.store(tid, Ordering::Release);
}

pub fn client_tid() -> u32 {
    CLIENT_TID.load(Ordering::Acquire)
}

/// 本线程的 TSF 管理器
pub fn thread_mgr() -> Option<ITfThreadMgr> {
    unsafe { CoCreateInstance(&crate::clsid::CLSID_TF_THREAD_MGR, None, CLSCTX_INPROC_SERVER).ok() }
}

/// 当前焦点文档的顶层 context
pub fn focus_context() -> Option<ITfContext> {
    let tim = thread_mgr()?;
    let doc = unsafe { tim.GetFocus() }.ok()?;
    unsafe { doc.GetTop() }.ok()
}

// ─────────────────────────────────────────────────────────────
// 中 / 英 开关
// ─────────────────────────────────────────────────────────────

/// 进程内的中英状态缓存。
///
/// 为什么不每次按键都去读 compartment：`ITfCompartment::GetValue` 是跨 COM 调用，
/// 按键路径上每次都要付一遍。更要紧的是——**宿主管不着这个开关**：实测切到英文后
/// CoreText/WinUI3 仍然把按键送进 TIP，所以必须由我们自己在 `OnKeyDown` 里判定。
/// 于是这个原子变量成了唯一的真相来源，compartment 只是同步给系统看的副本。
static CHINESE: AtomicBool = AtomicBool::new(true);

/// 读中英状态（进程内缓存，零 COM 开销）
pub fn chinese() -> bool {
    CHINESE.load(Ordering::Acquire)
}

/// 写中英开关：先改进程内缓存（立刻生效），再同步给 MSCTF，最后记进配置。
///
/// 返回 compartment 是否写成功——写失败不影响我们自己的判定，只是系统那边看不到。
///
/// **必须落盘**：中英状态是用户按过一次 Shift（或点了浮窗）的结果，不是临时态。
/// 不写盘的话，关掉程序再打开又回到默认值，用户会以为「切了没记住」——这正是
/// 用户报的现象。
pub fn set_chinese(open: bool) -> bool {
    CHINESE.store(open, Ordering::Release);
    let ok = write_compartment(open);
    crate::config::set_chinese(open);
    ok
}

/// 激活时用配置里的值初始化中英状态。
///
/// 和 `set_chinese` 的区别：**不写盘**。`Activate` 每个进程都会走一遍，如果这里
/// 也落盘，两个进程交替激活就会互相覆盖（A 存中文、B 存英文，谁最后激活算谁的）。
/// 配置只由「用户明确切换」来写。
pub fn init_chinese(open: bool) {
    CHINESE.store(open, Ordering::Release);
    write_compartment(open);
}

fn write_compartment(open: bool) -> bool {
    let tid = client_tid();
    if tid == 0 {
        return false;
    }
    let Some(tim) = thread_mgr() else {
        return false;
    };
    let Ok(mgr) = tim.cast::<ITfCompartmentMgr>() else {
        return false;
    };
    let Ok(comp) = (unsafe { mgr.GetCompartment(&GUID_COMPARTMENT_KEYBOARD_OPENCLOSE) }) else {
        return false;
    };
    let mut v = unsafe { VariantInit() };
    unsafe {
        // VARIANT_0.Anonymous 是 ManuallyDrop，必须显式解引用
        let inner = &mut *v.Anonymous.Anonymous;
        inner.vt = VT_I4;
        inner.Anonymous.lVal = i32::from(open);
    }
    unsafe { comp.SetValue(tid, &v) }.is_ok()
}

// ─────────────────────────────────────────────────────────────
// 插入文本
// ─────────────────────────────────────────────────────────────

/// 把一个 edit session 对象投到焦点 context 上同步执行。
fn run_session(session_iface: ITfEditSession) -> bool {
    let tid = client_tid();
    let Some(ctx) = focus_context() else {
        return false;
    };
    unsafe { ctx.RequestEditSession(tid, &session_iface, TF_ES_SYNC | TF_ES_READWRITE) }.is_ok()
}

#[implement(ITfEditSession)]
struct InsertSession {
    context: ITfContext,
    text: String,
}

impl InsertSession {
    fn new(context: ITfContext, text: String) -> Self {
        Self { context, text }
    }
}

impl ITfEditSession_Impl for InsertSession_Impl {
    fn DoEditSession(&self, ec: u32) -> Result<()> {
        unsafe {
            let mut sel = [TF_SELECTION::default()];
            let mut fetched: u32 = 0;
            self.context
                .GetSelection(ec, TF_DEFAULT_SELECTION, &mut sel, &mut fetched)?;
            if fetched == 0 {
                return Ok(());
            }
            let Some(range) = sel[0].range.take() else {
                return Ok(());
            };
            let wide: Vec<u16> = self.text.encode_utf16().collect();
            range.SetText(ec, 0, &wide)?;
            range.Collapse(ec, TF_ANCHOR_END)?;

            // 光标落到插入内容之后。TF_SELECTION.range 是 ManuallyDrop，
            // 两处引用都要手动 take 出来，否则会漏 AddRef/Release。
            drop(sel[0].range.take());
            sel[0].range.replace(range);
            let _ = self.context.SetSelection(ec, &sel);
            drop(sel[0].range.take());
        }
        Ok(())
    }
}

/// 在当前光标处插入一段文本（替换选区）。返回是否成功。
pub fn insert_text(text: &str) -> bool {
    if text.is_empty() || client_tid() == 0 {
        return false;
    }
    let Some(ctx) = focus_context() else {
        return false;
    };
    let session: ITfEditSession = InsertSession::new(ctx, text.to_string()).into();
    run_session(session)
}
