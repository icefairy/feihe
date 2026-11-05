// 飞鹤输入法 · DLL 入口与 COM 导出
// M1：注册用外部 installer（reg 脚本），DLL 只需 DllGetClassObject/DllCanUnloadNow
mod candidate;
mod classfactory;
mod clipboard;
mod clsid;
mod config;
mod custom;
mod datadir;
mod demote;
mod dict;
mod dictbin;
mod dictenc;
mod diag;
mod dllcount;
mod fancha;
mod font;
mod freq;
mod fullscreen;
mod hint;
mod insert;
mod next;
mod phrase;
mod pin;
mod quick;
mod secure;
mod settings;
mod softkbd;
mod sogou;
mod statusbar;
mod textservice;
mod theme;
mod uielement;
mod zhconv;

// ── 已经搬进 `feihe-core` 的平台无关模块 ────────────────────────────
//
// 挂回 crate 根之后，下面所有 `crate::shuangpin::…` 之类的**老路径一字不用改**。
// 搬家刻意不动调用点：真出了回归，才能确定是搬家本身的问题。
//
// 还在 `src/` 里的是下一批：`dictenc`（单测要扫 `src/*.txt` 做全量编码校验）、
// `dict` / `freq` / `next` / `pin` / `phrase` / `demote` / `custom`（`config` 已经
// 搬过去了，这批的下一个前置是把 `dictbin`/`dictenc`/`dict` 一起搬）。
pub use feihe_core::{assist, brief, brief2, calc, correct, fuzzy, number, shuangpin};

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
