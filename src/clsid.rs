// 飞鹤输入法 · 固定标识
// CLSID_FEIHE_TEXTSERVICE : 输入法服务 COM 类
// GUID_PROFILE_Feihe     : 语言栏 profile（输入法条目）
// GUID_DISPLAY_Feihe     : 显示属性（组合串下划线）

use windows::core::GUID;

pub const CLSID_FEIHE_TEXTSERVICE: GUID = GUID::from_u128(
    0x70cb823d_d08a_4b57_b8be_08bce2539ff3,
);

/// CLSID_TF_ThreadMgr —— 取「本线程的 TSF 管理器」。
///
/// 等价于 `TF_GetThreadMgr()`，但那个函数 windows-rs 没导出，所以直接
/// `CoCreateInstance`。文档明确：用这个 CLSID 拿到的就是**调用线程**的管理器。
pub const CLSID_TF_THREAD_MGR: GUID = GUID::from_u128(
    0x529a9e6b_6587_4f23_ab9e_9c7d683e3c50,
);

#[allow(dead_code)]
pub const GUID_PROFILE_FEIHE: GUID = GUID::from_u128(
    0xda6b9170_1db7_495f_8741_d2c6f3b5e28f,
);

#[allow(dead_code)]
pub const GUID_DISPLAY_FEIHE: GUID = GUID::from_u128(
    0xaabcddaf_01f1_4d7a_9326_cc4fafa5f652,
);

/// 输入法显示名
#[allow(dead_code)]
pub const FEIHE_NAME: &str = "飞鹤拼音";
/// 语言：简体中文（安装脚本按 0x00000804 写注册表，此处留作常量对照）
#[allow(dead_code)]
pub const LANGID_CHS: u16 = 0x0804;
