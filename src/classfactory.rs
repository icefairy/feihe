// COM 类工厂：CTF 用它创建 TextService 实例
use crate::textservice::{self, FeiheTextService};
use windows::core::*;
use windows_core::implement;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::TextServices::*;

#[implement(IClassFactory)]
pub struct FeiheClassFactory;

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
        unsafe { svc.query(riid, ppvobject) }
    }

    fn LockServer(&self, _flock: BOOL) -> Result<()> {
        Ok(())
    }
}

#[allow(dead_code)]
fn _type_check() {
    let _ = textservice::FeiheTextService::new();
}
