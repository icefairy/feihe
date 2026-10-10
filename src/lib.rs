// 飞鹤输入法 · DLL 入口与 COM 导出
// M1：注册用外部 installer（reg 脚本），DLL 只需 DllGetClassObject/DllCanUnloadNow
mod classfactory;
mod clsid;
mod textservice;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;

#[no_mangle]
extern "system" fn DllMain(
    _hinst: HINSTANCE,
    reason: u32,
    _reserved: *mut core::ffi::c_void,
) -> BOOL {
    let _ = reason == DLL_PROCESS_ATTACH; // Apartment 模型无需额外初始化
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
        let factory: IClassFactory = classfactory::FeiheClassFactory.into();
        factory.query(riid, ppv)
    }
}

#[no_mangle]
extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_OK
}
