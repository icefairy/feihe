//! 从搜狗**文件**导入个人词库：`.scel` 细胞词库 / `sgim_gd_usr*.bin` / 导出的 `.txt`。
//!
//! Windows 端这个入口在设置窗的「细胞词库…」按钮里，走的是
//! `feihe::sogou::import_file`——那边还要先用注册表找到搜狗安装目录。Android 上
//! 没有注册表，用户是从浏览器下载一个 `.scel` 然后交给系统文件选择器，所以这里
//! 只有「从一个文件路径走到个人词库」这一段。
//!
//! 解析和注音**不在这一层写**：`feihe_core::sogou::encode_one` 是两端共用的同一份
//! 实现，这一层只负责「读盘 → 调 core → 合并进个人词库 → 汇报数字」。

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

/// 真正的活：读文件 → 判格式 → 解析注音 → 合并 → 编译。抽出来方便单测。
pub fn import_path(path: &std::path::Path) -> String {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let Some(kind) = feihe_core::sogou::kind_for_ext(&ext) else {
        return format!(
            "ERR|不认识的扩展名 .{ext}（支持 .scel 细胞词库 / .bin / .txt 文本词库）"
        );
    };

    // `.scel` / `.bin` 动辄几十 MB，读之前先看一眼大小：手机的内存不像电脑那么宽裕，
    // 而这个词库本来也不该有几百 MB。
    match std::fs::metadata(path) {
        Ok(m) if m.len() > 128 * 1024 * 1024 => {
            return format!(
                "ERR|文件有 {} MB，太大了（词库文件不该超过 128 MB）",
                m.len() / 1024 / 1024
            )
        }
        Ok(_) => {}
        Err(e) => return format!("ERR|读不到这个文件：{e}"),
    }

    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => return format!("ERR|读不到这个文件：{e}"),
    };

    let label = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let job = feihe_core::sogou::Job {
        label,
        data,
        kind,
        // 手动挑的文件按细胞词库级（1）：和 Windows 端同一个取舍，见 sogou::kind_for_ext
        prio: 1,
    };

    let (wordlist, mut stat) = feihe_core::sogou::encode_one(&job, feihe_core::sogou::MAX_LEN);
    if stat.encoded == 0 {
        return format!("ERR|{}", stat.summary());
    }

    // 合并进个人词库（会顺手编译 user.bin；编译失败只影响速度，不算导入失败）。
    match feihe_core::dict::merge_text(&wordlist) {
        Ok((added, _bin)) => {
            stat.added = added;
            format!("OK|{}|{added}", stat.summary())
        }
        Err(e) => format!("ERR|解析出 {} 条，但写个人词库失败：{e}", stat.encoded),
    }
}
