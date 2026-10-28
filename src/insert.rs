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

/// 写中英开关：改进程内缓存（立刻生效），再把 MSCTF 的「键盘打开」状态拉起来。
///
/// 返回 compartment 是否写成功——写失败不影响我们自己的判定，只是系统那边看不到。
///
/// **不落盘**：这是会话内的临时状态。默认模式是设置项（`config.chinese`，设置窗
/// 「启动默认」），每次 Activate 从那里初始化；曾经把切换结果也写进配置，结果
/// 用户哪次按过 Shift 都被永久记住，表现就是「输入法默认是英文，不知道为什么」。
///
/// 注意这里**不看 `open`**：切到英文也不许去关 compartment，理由见
/// `keep_keyboard_open`。
pub fn set_chinese(open: bool) -> bool {
    CHINESE.store(open, Ordering::Release);
    keep_keyboard_open()
}

/// 激活时用配置里的「启动默认模式」初始化中英状态。
///
/// 这里**不写盘**：配置只由设置窗来写，进程内切换只改 `CHINESE` 缓存。每个进程
/// 的 Activate 都会走一遍，两个进程交替激活也不会互相覆盖。
///
/// 这里也照样把 compartment 拉到打开态：上次激活可能被写成了 0（老版本会这么干），
/// 不清掉的话这个进程会一直收不到按键。
pub fn init_chinese(open: bool) {
    CHINESE.store(open, Ordering::Release);
    keep_keyboard_open();
}

/// 把线程级的「键盘打开」compartment 置 1。
///
/// ## 只能写 1，永远不能写 0
///
/// `GUID_COMPARTMENT_KEYBOARD_OPENCLOSE` 不是我们的中英状态，它是**交给 MSCTF 的
/// 开关**：为 0 时 MSCTF 认为这个输入法处于英文态，**按键一律不派发给 TIP**，
/// `OnKeyDown` / `OnKeyUp` 一个都收不到。中英状态的真身是上面那个 `CHINESE` 原子量
/// （宿主不管 compartment，见 `CHINESE` 的注释）。
///
/// 曾经按 `open` 双向写：切英文时写 0，结果 Shift 的抬起事件再也送不进来，
/// 用户切到英文之后就切不回来了（报「Shift 无法切换中英文」），只能关掉应用重开。
/// 这个死锁没有别的出口——收不到键就没法再写回 1。
fn keep_keyboard_open() -> bool {
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
        inner.Anonymous.lVal = KEYBOARD_OPEN;
    }
    unsafe { comp.SetValue(tid, &v) }.is_ok()
}

/// 写进 `GUID_COMPARTMENT_KEYBOARD_OPENCLOSE` 的**唯一**合法值。
///
/// 单独拎成常量 + 下面的测试，是为了让「有人又想按中英状态双向写」这件事在
/// `cargo test` 里就撞墙，而不是等用户抱怨 Shift 切不回来。
const KEYBOARD_OPEN: i32 = 1;

#[cfg(test)]
mod tests {
    use super::*;

    /// 中英状态不许影响 MSCTF 的键盘开关：英文态下按键仍然必须派发给我们，
    /// 否则 Shift 抬起收不到，切回中文再没有入口（死锁）。
    #[test]
    fn keyboard_switch_stays_open_in_english_mode() {
        assert_eq!(KEYBOARD_OPEN, 1);
        // 真正的约束在 `keep_keyboard_open` 里：它不看 `CHINESE`，也不收参数。
        // 这里再钉一次「英文态下 CHINESE 是 false」这个前提，防止有人以为
        // 「英文态 = 关掉键盘开关」。
        CHINESE.store(false, Ordering::Release);
        assert!(!chinese());
        CHINESE.store(true, Ordering::Release);
    }
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
