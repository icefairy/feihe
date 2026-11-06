//! 从搜狗**文件**导入个人词库：`.scel` 细胞词库 / `sgim_gd_usr*.bin` / 导出的 `.txt`。
//!
//! Windows 端这个入口在设置窗的「细胞词库…」按钮里，走的是
//! `feihe::sogou::import_file`——那边还要先用注册表找到搜狗安装目录。Android 上
//! 没有注册表，用户是从浏览器下载一个 `.scel` 然后交给系统文件选择器，所以这里
//! 只有「从一个文件路径走到个人词库」这一段。
//!
//! **这一段现在是两端共用的**：`feihe_core::sogou_import::import_file` 干的就是
//! 「读盘 → 判扩展名 → 解析注音 → 合并进个人词库」，Windows 的「细胞词库…」按钮
//! 也调它（`src/settings.rs`）。以前这段逻辑各写一份，2026-11-07 才发现桌面端那份
//! 是**静默失败**的：`.scel` 解析出 0 条也回 `Ok`，界面只显示一句看不出问题的摘要。


use super::{make_string, read_string};
use jni::objects::{JClass, JString};
use jni::sys::jstring;
use jni::JNIEnv;

/// `NativeEngine.sogouImportNative(path)` —— 把搜狗词库文件并进个人词库。
///
/// 返回：
/// - `"OK|<summary>|<added>"`：`summary` 是给用户看的多行中文，`added` 是新增条数
///   （界面拿它决定要不要刷新「当前：…」那行小字）；
/// - `"ERR|<原因>"`：原因已经是人话了，界面直接显示。
///
/// 用 `|` 分隔而不是 JSON：这条路只有三个字段，为它引一个序列化库不值得，
/// 而 `summary` 里**不能出现 `|`**（中文摘要不会）。
#[no_mangle]
pub extern "system" fn Java_com_feihe_ime_engine_NativeEngine_sogouImportNative<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    path: JString<'local>,
) -> jstring {
    let path = read_string(&mut env, &path);
    let text = std::panic::catch_unwind(move || {
        if path.is_empty() {
            return "ERR|没有选择文件".to_string();
        }
        import_path(std::path::Path::new(&path))
    })
    .unwrap_or_else(|_| "ERR|导入时引擎内部出错".to_string());
    make_string(&mut env, &text)
}

/// 真正的活：只做「调 core 的共实现 + 拼返回串」。
///
/// **不在这一层读盘、不在这一层解析**：`feihe_core::sogou_import::import_file`
/// 两端共用（Windows 的「细胞词库…」也调它）。2026-11-07 之前这里和桌面端各写
/// 一份，结果桌面端那份解析出 0 条还返回成功，用户看到的就是「导入没反应」。
fn import_path(path: &std::path::Path) -> String {
    match feihe_core::sogou_import::import_file(path, feihe_core::sogou::MAX_LEN) {
        Ok(stat) => format!("OK|{}|{}", stat.summary(), stat.added),
        Err(e) => format!("ERR|{e}"),
    }
}
