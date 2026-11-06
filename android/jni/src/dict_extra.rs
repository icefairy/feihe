// 子模块：父模块（lib.rs）里的两个小工具在这里要用到。
use super::{make_string, read_string};
use jni::objects::{JClass, JString};
use jni::sys::jstring;
use jni::JNIEnv;

// ─────────────────────────────────────────────────────────────
// 个人词库：导入 / 导出 / 编译
// ─────────────────────────────────────────────────────────────
//
// Windows 端设置窗里有三个按钮（导入词库 / 导出词库 / 编译个人词库），对应
// core 里三个早就写好的函数。Android 端一直没有入口——用户既导不进自己的词表，
// 也带不走自己在手机上打的词。
//
// **格式两端同一份**（`词<TAB>全拼`），所以「电脑上导出的词库导入到手机」是
// 直接能用的，这是这两个入口最大的意义。
//
// 文件路径由 Kotlin 给（系统文件选择器的结果），这一层只做搬运：读文件、调 core、
// 把条目数/错误信息变回字符串。core 里那两个函数自己会写盘、会编译 user.bin。

/// `NativeEngine.dictExportNative(path)` —— 导出全部词库到文本。
///
/// 返回 `"OK <条目数>"` 或 `"ERR <原因>"`。前缀而不是异常：JNI 里抛 Java 异常要
/// 建 Throwable 对象，而 Kotlin 侧读一个前缀字符串就够了，出错路径也不值得再
/// 加一条 `UnsatisfiedLinkError` 之外的可能性。
#[no_mangle]
pub extern "system" fn Java_com_feihe_ime_engine_NativeEngine_dictExportNative<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    path: JString<'local>,
) -> jstring {
    let path = read_string(&mut env, &path);
    let text = std::panic::catch_unwind(move || {
        if path.is_empty() {
            return "ERR 没有选择文件".to_string();
        }
        match feihe_core::dict::export_to(std::path::Path::new(&path)) {
            Ok(n) => format!("OK {n}"),
            Err(e) => format!("ERR {e}"),
        }
    })
    .unwrap_or_else(|_| "ERR 导出时引擎内部出错".to_string());
    make_string(&mut env, &text)
}

/// `NativeEngine.dictImportNative(path)` —— 把一份 `词<TAB>全拼` 并进个人词库。
///
/// 返回 `"OK <新增条目数>"` 或 `"ERR <原因>"`。**追加不是覆盖**：用户可能同时
/// 从搜狗和从电脑各导一份，覆盖会神秘地吃掉前一份。
///
/// core 那边 `merge_text` 会顺手编译 `user.bin`；编译失败只影响速度（退回文本），
/// 导入本身仍然是成功的，所以这里不把编译状态当错误。
#[no_mangle]
pub extern "system" fn Java_com_feihe_ime_engine_NativeEngine_dictImportNative<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    path: JString<'local>,
) -> jstring {
    let path = read_string(&mut env, &path);
    let text = std::panic::catch_unwind(move || {
        if path.is_empty() {
            return "ERR 没有选择文件".to_string();
        }
        match feihe_core::dict::import_from(std::path::Path::new(&path)) {
            // 第二个返回值是「user.bin 编译结果」：`None` 表示编译没做成（退回文本
            // 查询，能用但慢一点），不是导入失败。所以只要导入本身 OK，就是 OK。
            Ok((added, _bin)) => format!("OK {added}"),
            Err(e) => format!("ERR {e}"),
        }
    })
    .unwrap_or_else(|_| "ERR 导入时引擎内部出错".to_string());
    make_string(&mut env, &text)
}

/// `NativeEngine.dictCompileNative()` —— 把 `user_dict.txt` 编译成 `user.bin`。
///
/// 返回 `"OK <条目数> <字节数>"` 或 `"ERR <原因>"`。
///
/// 这条在 Android 上比 Windows 更有意义：手机上没法手改 `user_dict.txt`，
/// 但导入后如果编译失败就会一直走文本回退——把「到底编没编成」亮出来，
/// 用户才知道要不要重导。
#[no_mangle]
pub extern "system" fn Java_com_feihe_ime_engine_NativeEngine_dictCompileNative<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jstring {
    let text = std::panic::catch_unwind(|| match feihe_core::dict::compile_user_bin() {
        Ok(feihe_core::dict::CompileOutcome::Compiled { count, bytes }) => {
            format!("OK {count} {bytes}")
        }
        // 「没有 user_dict.txt」不是失败，是「本来就没导入过」，得给句人话。
        Ok(feihe_core::dict::CompileOutcome::NoSource { existing }) => match existing {
            Some(n) => format!("OK 0 {n} 条已在上次编好的 user.bin 里"),
            None => "ERR 还没有个人词库（先导入一份）".to_string(),
        },
        Err(e) => format!("ERR {e}"),
    })
    .unwrap_or_else(|_| "ERR 编译时引擎内部出错".to_string());
    make_string(&mut env, &text)
}

/// `NativeEngine.dictSourceNative()` —— 个人词库当前**实际生效**的来源。
///
/// 界面拿它显示一行小字：「user.bin 已启用」/「文本回退」/「未导入」。
/// 没有这一行，用户导完词库发现候选没变，唯一能做的就是重装。
#[no_mangle]
pub extern "system" fn Java_com_feihe_ime_engine_NativeEngine_dictSourceNative<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jstring {
    let text = std::panic::catch_unwind(feihe_core::dict::user_source_label).unwrap_or_default();
    make_string(&mut env, &text)
}
