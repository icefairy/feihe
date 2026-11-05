//! 飞鹤输入法 · Android 的 JNI 桥。
//!
//! 这一层**只做搬运**：把 Kotlin 的字符串变成 Rust 的 `&str`，调 `feihe-core`，
//! 再把结果变回字符串。不放任何业务逻辑——判断、排序、编码规则全在 core 里，
//! 那边有 200 多项单测盯着，这边没有。
//!
//! ## 三个刻意的设计
//!
//! **一、每个入口都用 `catch_unwind` 包着。** 一个 Rust panic 如果穿过 FFI 边界，
//! 结果是整个进程 abort——在输入法里这意味着用户正在写的文档没了。所以宁可让
//! 这次调用返回空、记日志，也不能连带杀死宿主 App。
//!
//! **二、宁可返回空字符串，也不返回 null。** Kotlin 侧拿到 null 要处处判空，
//! 漏一处就是 NPE；空串在「没有候选 / 编码失败」这个语义上是准确的，调用方
//! 按「空就是没有」处理就够了。
//!
//! **三、函数名就是 JNI 符号名。** 没有 `@JvmStatic` 之类的花活，Kotlin 那边
//! 写 `external fun` 时**类名和包名必须一字不差**——改了包名却忘了改这里，
//! 表现为 `UnsatisfiedLinkError`，而这只有装到真机上才会发现。

use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jstring};
use jni::JNIEnv;

/// 建一个 Java 字符串。失败（OOM / JNI 异常）时返回 null——这是唯一该返回 null
/// 的场合，因为此时确实「没有字符串可给」。
fn make_string<'local>(env: &mut JNIEnv<'local>, s: &str) -> jstring {
    match env.new_string(s) {
        Ok(v) => v.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// 把 `JString` 读成 Rust 字符串。读失败就当空串——对编码函数来说「空输入」
/// 和「读不出来」的结论一样：没有结果。
fn read_string(env: &mut JNIEnv, s: &JString) -> String {
    match env.get_string(s) {
        Ok(v) => v.to_string_lossy().into_owned(),
        Err(_) => String::new(),
    }
}

/// `NativeEngine.coreVersion()`（Kotlin 侧的包装，原生方法名带 `Native` 后缀）
///
/// 给界面和日志用：能打出这一行，就证明 `.so` 加载成功、JNI 符号对得上、
/// 字符串能双向过边界。**这是整条链路的第一个可验证点**，所以它必须在。
#[no_mangle]
pub extern "system" fn Java_com_feihe_ime_engine_NativeEngine_coreVersionNative<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jstring {
    let text = std::panic::catch_unwind(|| {
        format!("{} ({})", feihe_core::VERSION, std::env::consts::ARCH)
    })
    .unwrap_or_else(|_| "panic".to_string());
    make_string(&mut env, &text)
}

/// `NativeEngine.encodePinyin(pinyin)` —— 全拼 → 小鹤双拼码。
///
/// 为什么先搬这一个函数上去：它是**纯函数里最像真活的那个**。
/// `nihao` → `nihc`、`zhuang` → `vl`，一眼能看出对错，不依赖词库、不依赖状态。
/// 拿它验证 JNI 边界，比拿一个 `add(a,b)` 有意义得多。
///
/// 编码不出来（不是合法拼音、或者超出双拼能表达的音节）返回空串。
#[no_mangle]
pub extern "system" fn Java_com_feihe_ime_engine_NativeEngine_encodePhraseNative<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    pinyin: JString<'local>,
) -> jstring {
    let input = read_string(&mut env, &pinyin);
    let code = std::panic::catch_unwind(|| {
        feihe_core::shuangpin::encode_phrase(feihe_core::shuangpin::Scheme::Xiaohe, &input)
            .unwrap_or_default()
    })
    .unwrap_or_default();
    make_string(&mut env, &code)
}

/// `NativeEngine.isSyllable(text)` —— 是不是一个完整合法的拼音音节。
///
/// 引擎接进来之后，键盘要用它决定「现在按空格是选词还是原样上屏」。先放上去，
/// 让 Kotlin 侧那套「什么时候该问引擎」的代码路径能提前跑通。
#[no_mangle]
pub extern "system" fn Java_com_feihe_ime_engine_NativeEngine_isSyllableNative<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    text: JString<'local>,
) -> jboolean {
    let input = read_string(&mut env, &text);
    let ok = std::panic::catch_unwind(|| feihe_core::shuangpin::is_syllable(&input)).unwrap_or(false);
    u8::from(ok)
}

/// `NativeEngine.initNative(dataDir, utcOffsetMinutes)`
///
/// 引擎启动时必须先调这个，否则数据目录是空的（core 的 `paths::dir()` 会 panic）。
/// `utcOffsetMinutes` 给 `custom` 的 `{hour}` 之类变量用——Android 的 Rust 侧拿不到
/// 时区，只能从 Kotlin 的 `TimeZone.getDefault().getOffset(now) / 60000` 传进来。
#[no_mangle]
pub extern "system" fn Java_com_feihe_ime_engine_NativeEngine_initNative<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    data_dir: JString<'local>,
    utc_offset_minutes: i32,
) {
    let dir = read_string(&mut env, &data_dir);
    let _ = std::panic::catch_unwind(|| {
        feihe_core::paths::init(std::path::PathBuf::from(dir));
        feihe_core::clock::set_utc_offset_minutes(utc_offset_minutes);
        // Android 首次没有 config.ini，这里会得到默认值；有文件就按文件来。
        // 不调这一步的话 `pinyin_mode` / `candidate_rows` 这些原子镜像都是初值，
        // 用户在设置里改过的选项不会生效。
        feihe_core::config::refresh_if_changed();
    });
}

/// `NativeEngine.lookupNative(code, limit)` —— 按双拼码查候选，`\n` 分隔。
///
/// 只查**一个码**，不做简繁转换、不做联想、不记录词频：那些是壳的策略（用哪个码、
/// 什么时候学），不属于「查词库」这件事。Android 第一版先要「按了键能出字」。
#[no_mangle]
pub extern "system" fn Java_com_feihe_ime_engine_NativeEngine_lookupNative<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    code: JString<'local>,
    limit: i32,
) -> jstring {
    let code = read_string(&mut env, &code);
    let text = std::panic::catch_unwind(move || {
        if code.is_empty() {
            return String::new();
        }
        // 上限放在这里而不是信调用方：候选窗再宽也放不下几十个，
        // 而一次查太多会白白穿越几十次 JNI 字符串边界。
        let n = limit.clamp(1, 60) as usize;
        feihe_core::dict::lookup(&code, n).join("\n")
    })
    .unwrap_or_default();
    make_string(&mut env, &text)
}

/// `NativeEngine.suggestNative(prev, limit)` —— `prev` 后面可能接的词，`\n` 分隔。
///
/// 和 `lookupNative` 同一个约定（`\n` 分隔、空串表示没有）。按「用户表优先、
/// 内置表补位」合并，这个顺序在 core 里定死了，壳不要重排——用户自己打过的词
/// 对他永远比词库统计更可信。
///
/// 这里**不记录任何东西**。学习发生在上屏那一刻（`recordNative`），联想只是读。
#[no_mangle]
pub extern "system" fn Java_com_feihe_ime_engine_NativeEngine_suggestNative<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    prev: JString<'local>,
    limit: i32,
) -> jstring {
    let prev = read_string(&mut env, &prev);
    let text = std::panic::catch_unwind(move || {
        if prev.is_empty() {
            return String::new();
        }
        // 手机候选条一行放不下几个，上限比 `lookup` 更紧
        let n = limit.clamp(1, 12) as usize;
        feihe_core::next::suggest(&prev, n).join("\n")
    })
    .unwrap_or_default();
    make_string(&mut env, &text)
}

/// `NativeEngine.recordNative(prev, next)` —— 记一对「上次上屏 → 这次上屏」。
///
/// 写盘由 core 的 `maybe_flush` 自己节流，所以每个词都可以调一次 —— 手机上
/// 「每次上屏都 fsync」是不可接受的，而这件事不该由壳来判断（否则每个壳都会
/// 各写一套节流规则，然后不一致）。
#[no_mangle]
pub extern "system" fn Java_com_feihe_ime_engine_NativeEngine_recordNative<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    prev: JString<'local>,
    next: JString<'local>,
) {
    let prev = read_string(&mut env, &prev);
    let next = read_string(&mut env, &next);
    let _ = std::panic::catch_unwind(|| {
        feihe_core::next::record(&prev, &next);
    });
}
