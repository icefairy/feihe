// 子模块：父模块（lib.rs）里的两个小工具在这里要用到。
use super::{make_string, read_string};
use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jstring};
use jni::JNIEnv;

// ─────────────────────────────────────────────────────────────
// 快捷输入 / 个人词库：给 Android 用的四个入口
// ─────────────────────────────────────────────────────────────
//
// 这两摊东西 Windows 端早就有，Android 端一直没有——不是做不到，而是它们原来
// 长在壳里（`src/quick.rs` 调 `GetLocalTime`、词库导入导出是设置窗的 `if`）。
// `quick` 搬进 core 之后（一行系统调用不剩），两边终于能共用同一份。

/// `NativeEngine.quickPrefixNative()` —— 快捷输入的引导符（`/`）。
///
/// 为什么让 Kotlin 来问而不是自己写死一个 `/`：引导符是**引擎的语法**，
/// 壳里再写一份常量，哪天引擎改成别的符号，Android 就会静默地不支持快捷输入。
#[no_mangle]
pub extern "system" fn Java_com_feihe_ime_engine_NativeEngine_quickPrefixNative<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jstring {
    let text = std::panic::catch_unwind(|| feihe_core::quick::PREFIX.to_string())
        .unwrap_or_default();
    make_string(&mut env, &text)
}

/// `NativeEngine.quickCandidatesNative()` —— 只打一个 `/` 时的预览行，`\n` 分隔。
///
/// 对应 Windows 端「敲一个 `/` 就列出所有命令当前值」的行为：不知道码的用户
/// 一眼看到能打什么，按哪个数字就拿哪一项。
#[no_mangle]
pub extern "system" fn Java_com_feihe_ime_engine_NativeEngine_quickCommandsNative<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jstring {
    let text = std::panic::catch_unwind(|| feihe_core::quick::candidates("").join("\n"))
        .unwrap_or_default();
    make_string(&mut env, &text)
}

/// `NativeEngine.quickCandidatesNative(key, limit)` —— `/` 后面的码 → 候选，`\n` 分隔。
///
/// `key` **不含引导符**（Kotlin 那边已经把 `/` 切掉了），这一点和 Windows 端
/// `textservice` 的调用方式一致：引导符是壳的语法糖，`quick` 只认后面的码。
///
/// 空串也是合法输入（等于「只打了 `/`」），返回全部命令的预览——这正是只敲 `/`
/// 时想要的东西，所以这里**不要**把空串当错误提前返回。
#[no_mangle]
pub extern "system" fn Java_com_feihe_ime_engine_NativeEngine_quickCandidatesNative<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    key: JString<'local>,
    limit: i32,
) -> jstring {
    let key = read_string(&mut env, &key);
    let text = std::panic::catch_unwind(move || {
        let n = limit.clamp(1, 60) as usize;
        let mut v = feihe_core::quick::candidates(&key);
        v.truncate(n);
        v.join("\n")
    })
    .unwrap_or_default();
    make_string(&mut env, &text)
}

/// `NativeEngine.quickCodeOkNative(key)` —— `/` 后面的码当前是不是合法中间态。
///
/// 键盘用它决定「这一下按键吃不吃进组合串」。必须支持**中间态**
/// （`/uuid` 会经过 `u`、`uu`、`uui`），否则用户看到的是「打一半字消失」。
#[no_mangle]
pub extern "system" fn Java_com_feihe_ime_engine_NativeEngine_quickCodeOkNative<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    key: JString<'local>,
) -> jboolean {
    let key = read_string(&mut env, &key);
    let ok = std::panic::catch_unwind(move || feihe_core::quick::code_ok(&key)).unwrap_or(false);
    u8::from(ok)
}
