// 飞鹤输入法 · DLL 入口与 COM 导出
// M1：注册用外部 installer（reg 脚本），DLL 只需 DllGetClassObject/DllCanUnloadNow
mod candidate;
mod classfactory;
mod clipboard;
mod clsid;
mod config;
mod datadir;
mod diag;
mod dpi;
mod dllcount;
mod font;
mod fullscreen;
mod insert;
mod nls;
mod quick;
mod secure;
mod settings;
mod softkbd;
mod sogou;
mod statusbar;
mod textservice;
mod theme;
mod uielement;

// ── 已经搬进 `feihe-core` 的平台无关模块 ────────────────────────────
//
// 挂回 crate 根之后，下面所有 `crate::shuangpin::…` 之类的**老路径一字不用改**。
// 搬家刻意不动调用点：真出了回归，才能确定是搬家本身的问题。
//
// 留在 `src/` 的是**真的离不开 Windows 的**：`sogou`（注册表找搜狗目录 + 导入）、
// `clipboard`（剪贴板 API）、`fullscreen`（QUNS）、`nls`（NLS 简繁表）、UI 与 TSF
// 那一堆。core 里凡是需要这些能力的（`zhconv` 的映射、`fancha` 读剪贴板）都改成
// 壳注入的钩子，见各自的模块注释。
pub use feihe_core::{
    assist, brief, brief2, calc, correct, custom, demote, dict, dictbin, dictenc, fancha, freq,
    fuzzy, hint, next, number, phrase, pin, shuangpin, zhconv,
};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;

/// 交给 core 的全屏探测器（`config::PopupMode::Auto` 用）。
///
/// 写成独立的 `fn` 而不是内联闭包，是为了让「这是壳提供的策略」这件事在类型上
/// 就是明确的——`set_fullscreen_probe` 收的就是一个裸函数指针。
fn popup_probe() -> bool {
    fullscreen::covers_popup(fullscreen::scene())
}

#[no_mangle]
extern "system" fn DllMain(
    _hinst: HINSTANCE,
    reason: u32,
    _reserved: *mut core::ffi::c_void,
) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        // 只注册策略，**不执行**：`datadir::resolve` 会试写文件、做首次迁移，
        // 那是文件 I/O，在 loader lock 里做会卡住甚至死锁。core 会在第一次真正
        // 需要数据目录时惰性调用它。
        feihe_core::paths::set_resolver(datadir::resolve);
        config::set_fullscreen_probe(popup_probe);
        // 简繁映射（NLS）和剪贴板都属于「只有 Windows 有实现」的能力，
        // 同样只登记函数指针——真正的系统调用发生在用户按下繁体开关 / 敲 `/fc` 时。
        feihe_core::zhconv::set_converter(nls::to_traditional);
        feihe_core::fancha::set_clipboard_reader(clipboard::text);
    }
    BOOL::from(true)
}

/// CTF 框架 CoCreateInstance(CLSID_FEIHE) 创建输入法
#[no_mangle]
extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut core::ffi::c_void,
) -> HRESULT {
    unsafe {
        if ppv.is_null() || rclsid.is_null() || riid.is_null() {
            return E_POINTER;
        }
        if *rclsid != clsid::CLSID_FEIHE_TEXTSERVICE {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: IClassFactory = classfactory::FeiheClassFactory::new().into();
        let hr = factory.query(riid, ppv);
        hr
    }
}

/// 是否可以卸载 DLL。
/// 只要有活对象（类工厂 / 文本服务）或 LockServer 锁，就必须返回 S_FALSE，
/// 否则 MSCTF 仍持有本 DLL 内的指针时 DLL 被卸载 -> use-after-unload -> 宿主 AV 崩溃。
#[no_mangle]
extern "system" fn DllCanUnloadNow() -> HRESULT {
    if dllcount::can_unload() {
        S_OK
    } else {
        S_FALSE
    }
}
