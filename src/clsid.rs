// 飞鹤输入法 · 固定标识
// CLSID_TeiheTextService : 输入法服务 COM 类
// GUID_PROFILE_Feihe     : 语言栏 profile（输入法条目）
// GUID_DISPLAY_Feihe     : 显示属性（组合串下划线）

use windows::core::GUID;

pub const CLSID_FEIHE_TEXTSERVICE: GUID = GUID::from_u128(
    0x70cb823d_d08a_4b57_b8be_08bce2539ff3,
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
/// 语言：简体中文
pub const LANGID_CHS: u16 = 0x0804;
