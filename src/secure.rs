//! 密码框识别：在密码输入框里一个键都不许拦。
//!
//! 用户报的现象很具体：某些登录框、支付密码框里打字，输入法照样弹候选，一按空格
//! 就把中文写了进去——密码框里出现「你好」两个字，等于这次登录已经废了（而且多半
//! 已经输错了，用户还不知道为什么登不上去）。
//!
//! 这不是「要不要弹窗」的策略问题，是**权限问题**：密码框里的每一次按键都只属于
//! 用户和那个应用，输入法连看都不该看。所以这里不是「不弹候选」而是「根本不接管」——
//! `OnKeyDown` 直接返回 FALSE，键原样进应用。
//!
//! ## 为什么两套判据
//!
//! 1. **TSF input scope**（`IS_PASSWORD`）：现代宿主（Chromium/Electron、UWP、WinUI、
//!    WPF 新版）走这条。这是规范做法。
//! 2. **窗口样式 `ES_PASSWORD`**：经典 Win32 编辑框（记事本那种 `Edit`、老式
//!    `RichEdit`）压根不设 input scope，但它们的窗口样式里有这一位。只靠 (1) 的话，
//!    最古老、最需要保护的那批程序反而漏掉。
//!
//! 两套是「或」的关系：任一命中就当密码框。
//!
//! ## 为什么探测要开一个同步编辑会话
//!
//! `ITfReadOnlyProperty::GetValue` 要 edit cookie，而 `ITfKeyEventSink::OnKeyDown`
//! 的参数里没有 ec。所以只能 request 一个 `TF_ES_SYNC | TF_ES_READ` 的会话，在
//! `DoEditSession(ec)` 里读。
//!
//! 这个开销不能付在每次按键上，所以调用方（`textservice`）按 context 指针缓存结果：
//! 同一个输入位置只探一次，换焦点才重探。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use windows::core::*;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Variant::{VariantClear, VT_UNKNOWN};
use windows::Win32::UI::TextServices::*;
use windows::Win32::UI::WindowsAndMessaging::{GetWindowLongW, GWL_STYLE};
use windows_core::implement;

/// 经典编辑框的密码位（`winuser.h` 里 `ES_PASSWORD`）。
///
/// 没有走 windows crate 的常量是因为它躲在 `Win32_UI_Controls` 下，为一个 0x20
/// 再开一个 feature 不值得（安装包体积是产品特性）。
const ES_PASSWORD: u32 = 0x0020;

/// 这个输入位置是不是密码框。
///
/// 任何一步失败都返回 `false`：探不出来就当普通输入框，宁可按老行为来，也不能
/// 因为一次 COM 调用失败就让用户在普通框里打不出字。
pub fn is_password(ctx: &ITfContext) -> bool {
    let tid = crate::insert::client_tid();
    if tid == 0 {
        return false;
    }
    // 窗口样式这条不需要 edit cookie，先问它——命中就不用开会话了
    if unsafe { has_password_style(ctx) } {
        return true;
    }

    let flag = Arc::new(AtomicBool::new(false));
    let session: ITfEditSession = Probe {
        scope: Arc::new(ScopeProbe {
            ctx: ctx.clone(),
            out: flag.clone(),
        }),
    }
    .into();
    if unsafe { ctx.RequestEditSession(tid, &session, TF_ES_SYNC | TF_ES_READ) }.is_err() {
        return false;
    }
    flag.load(Ordering::Acquire)
}

/// `ES_PASSWORD` 兜底判据。
unsafe fn has_password_style(ctx: &ITfContext) -> bool {
    let Ok(view) = ctx.GetActiveView() else {
        return false;
    };
    let Ok(hwnd) = view.GetWnd() else {
        return false;
    };
    if hwnd.is_invalid() {
        return false;
    }
    // GetWindowLongW 对非本线程窗口也有效（跨进程读窗口样式是允许的）
    let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
    style & ES_PASSWORD != 0
}

/// 只在 `DoEditSession` 里用到的组合体。
///
/// `#[implement]` 生成的对象要求字段能跨线程（`Send + Sync`），所以用 `Arc` 包一层
/// 而不是 `Cell`——那个 `Arc<AtomicBool>` 就是调用方读结果的口子。
#[implement(ITfEditSession)]
struct Probe {
    scope: Arc<ScopeProbe>,
}

struct ScopeProbe {
    ctx: ITfContext,
    out: Arc<AtomicBool>,
}

impl ITfEditSession_Impl for Probe_Impl {
    fn DoEditSession(&self, ec: u32) -> Result<()> {
        let hit = unsafe { scope_has_password(&self.scope.ctx, ec) };
        self.scope.out.store(hit, Ordering::Release);
        Ok(())
    }
}

/// 读 `GUID_PROP_INPUTSCOPE`，看里面有没有 `IS_PASSWORD`。
unsafe fn scope_has_password(ctx: &ITfContext, ec: u32) -> bool {
    let Ok(prop) = ctx.GetAppProperty(&GUID_PROP_INPUTSCOPE) else {
        return false;
    };
    // prange = None：这个属性是「整个 context 的」，不针对某一段文本
    let Ok(mut var) = prop.GetValue(ec, None::<&ITfRange>) else {
        return false;
    };

    let mut found = false;
    {
        let inner = &*var.Anonymous.Anonymous;
        // VT_UNKNOWN 里装着 ITfInputScope；其它类型说明宿主没设过 input scope
        if inner.vt == VT_UNKNOWN {
            if let Some(unk) = &*inner.Anonymous.punkVal {
                // cast 成功才说明它真是 input scope（有些宿主往这个属性里塞别的东西）
                if let Ok(scope) = unk.cast::<ITfInputScope>() {
                    found = scopes_contain_password(&scope);
                }
            }
        }
    }
    // VARIANT 是 ManuallyDrop 语义，必须自己清（punkVal 的引用计数在里面）
    let _ = VariantClear(&mut var);
    found
}

unsafe fn scopes_contain_password(scope: &ITfInputScope) -> bool {
    let mut arr: *mut InputScope = core::ptr::null_mut();
    let mut n: u32 = 0;
    if scope.GetInputScopes(&mut arr, &mut n).is_err() || arr.is_null() || n == 0 {
        if !arr.is_null() {
            CoTaskMemFree(Some(arr as *const core::ffi::c_void));
        }
        return false;
    }
    // 这几个值来自 CoTaskMemAlloc，读的时候借用，读完统一释放
    let list = core::slice::from_raw_parts(arr, n as usize);
    let found = list.iter().any(|s| s.0 == IS_PASSWORD.0);
    CoTaskMemFree(Some(arr as *const core::ffi::c_void));
    found
}

#[cfg(test)]
mod tests {
    /// `ES_PASSWORD` 这个魔数写错了就等于这道兜底完全没有——而它错不错**永远不会
    /// 在正常使用里表现出来**（只是"密码框还是会被输入法接管"）。用一张真实的
    /// 样式位定义钉住它：`winuser.h` 里 `ES_PASSWORD` 是 `0x0020L`。
    #[test]
    fn es_password_bit_is_right() {
        assert_eq!(super::ES_PASSWORD, 0x20, "ES_PASSWORD 必须是 0x20");
        // 相邻的样式位不能误伤：ES_AUTOHSCROLL=0x80、ES_READONLY=0x800
        assert_eq!(super::ES_PASSWORD & 0x80, 0);
        assert_eq!(super::ES_PASSWORD & 0x800, 0);
    }
}
