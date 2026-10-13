// COM 类工厂：CTF 用它创建 TextService 实例
use crate::dllcount;
use crate::textservice::{self, FeiheTextService};
use windows::core::*;
use windows_core::implement;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::TextServices::*;

#[implement(IClassFactory)]
pub struct FeiheClassFactory;

impl FeiheClassFactory {
    /// 创建并登记一个类工厂对象（DLL 计数 +1）
    pub fn new() -> Self {
        dllcount::object_created();
        Self
    }
}

impl Drop for FeiheClassFactory {
    fn drop(&mut self) {
        dllcount::object_destroyed();
    }
}

impl IClassFactory_Impl for FeiheClassFactory_Impl {
    fn CreateInstance(
        &self,
        punkouter: Ref<'_, IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut core::ffi::c_void,
    ) -> Result<()> {
        // 输入法服务不支持聚合
        if punkouter.is_some() {
            return Err(Error::new(
                CLASS_E_NOAGGREGATION,
                "feihe: no aggregation",
            ));
        }
        let svc: ITfTextInputProcessorEx = FeiheTextService::new().into();
        unsafe { svc.query(riid, ppvobject).ok()?; }
        Ok(())
    }

    fn LockServer(&self, flock: BOOL) -> Result<()> {
        dllcount::lock_server(flock.as_bool());
        Ok(())
    }
}

#[allow(dead_code)]
fn _type_check() {
    let _ = textservice::FeiheTextService::new();
}
