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
        // **方案必须取当前的，不能写死 Xiaohe。** 用户在设置里切到微软双拼之后，
        // 这里还按小鹤编码的话，每个音节的韵母位置都是错的——而界面上看不出来，
        // 只会表现为「候选莫名其妙不对」。
        feihe_core::shuangpin::encode_phrase(feihe_core::dict::scheme(), &input).unwrap_or_default()
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
        // 方案要同步进 `dict` 的原子量：`dict::scheme()` 是查词库时用的那份，
        // 和 `config.scheme` 是两处状态。Windows 端在 `Activate` 里做，Android 没有
        // 那个时机，只能在 init 里做一次 + 设置里改的时候再做一次。
        feihe_core::dict::set_scheme(feihe_core::config::get().scheme);
    });
}

/// `NativeEngine.lookupInputNative(input, pinyinMode, limit)` —— 按键串 → 码 + 候选。
///
/// 返回串的**第一行是「实际用的码」**，后面每行一个候选；空串表示「没有码，也没有
/// 候选」。用一行塞码是为了**一次 JNI 调用**解决问题：码要显示在状态条上（`nihao→nihc`），
/// 候选要摆进候选条，分两次调用就意味着同一串字母要过两遍边界、两遍切分。

/// 若拼出了整句，末尾再跟一个 `#seg` 标记行和分段数据（每行 `段码	候选…`）。以前分段由壳拿到结果后再调一次 JNI
/// 重算——同一段 DP 每次按键跑两遍；现在 core 算好的结果直接搭车回来。
///
/// 判断「这串按键是双拼码还是全拼」的逻辑在 core 的 `dict::lookup_input` 里，
/// 不在这里——见那边的注释：它是第二份真值的高危区，必须在有单测的地方。
#[no_mangle]
pub extern "system" fn Java_com_feihe_ime_engine_NativeEngine_lookupInputNative<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    input: JString<'local>,
    pinyin_mode: jboolean,
    limit: i32,
) -> jstring {
    let input = read_string(&mut env, &input);
    let text = std::panic::catch_unwind(move || {
        if input.is_empty() {
            return String::new();
        }
        let n = limit.clamp(1, 60) as usize;
        let r = feihe_core::dict::lookup_input(&input, pinyin_mode != 0, n);
        if r.code.is_empty() {
            return String::new();
        }
        let mut out = r.code;
        for w in r.words {
            out.push('\n');
            out.push_str(w.as_ref());
        }
        if !r.segments.is_empty() {
            out.push_str("\n#seg");
            for seg in r.segments {
                out.push('\n');
                out.push_str(&seg.code);
                for w in seg.words {
                    out.push('\t');
                    out.push_str(&w);
                }
            }
        }
        out
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

/// `NativeEngine.configNative()` —— 设置页要显示的开关快照，`key=value;…`。
///
/// 用「自己拼串」而不是 JSON：这里只有五个布尔/枚举，引序列化库得不偿失，
/// 而且拼串的格式错一眼能看出来。值和 `config.ini` 里一致（`xiaohe` / `ms`、`0` / `1`），
/// 设置页原样回写。
#[no_mangle]
pub extern "system" fn Java_com_feihe_ime_engine_NativeEngine_configNative<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jstring {
    let s = std::panic::catch_unwind(|| {
        use feihe_core::shuangpin::Scheme;
        let c = feihe_core::config::get();
        let scheme = match c.scheme {
            Scheme::Xiaohe => "xiaohe",
            Scheme::Ms => "ms",
        };
        format!(
            "scheme={scheme};pinyin={};traditional={};suggest={};learn={};xuanxue={}",
            u8::from(c.pinyin_mode),
            u8::from(c.traditional),
            u8::from(c.suggest),
            u8::from(c.learn),
            u8::from(c.xuanxue),
        )
    })
    .unwrap_or_default();
    make_string(&mut env, &s)
}

/// `NativeEngine.setConfigNative(key, value)` —— 改一项设置并落盘。
///
/// 只认下面这几个 key，别的一律忽略：设置页能改的东西是白名单，不能让界面
/// 随便往 `config.ini` 里塞字段（那些字段 Windows 端还在读，塞坏了那边不认）。
///
/// **改方案必须同步 `dict::set_scheme`**——`config.scheme` 和 `dict::scheme()`
/// 是两处状态，只改前者会出现「设置里显示微软双拼、查词还按小鹤」，
/// 而这种错在界面上完全看不出来，只表现为候选莫名其妙不对。
#[no_mangle]
pub extern "system" fn Java_com_feihe_ime_engine_NativeEngine_setConfigNative<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    key: JString<'local>,
    value: JString<'local>,
) {
    let k = read_string(&mut env, &key);
    let v = read_string(&mut env, &value);
    let _ = std::panic::catch_unwind(|| {
        use feihe_core::shuangpin::Scheme;
        let on = v == "1";
        match k.as_str() {
            "scheme" => {
                let s = if v == "ms" { Scheme::Ms } else { Scheme::Xiaohe };
                feihe_core::config::update(|c| c.scheme = s);
                feihe_core::dict::set_scheme(s);
            }
            "pinyin" => feihe_core::config::update(|c| c.pinyin_mode = on),
            "traditional" => feihe_core::config::update(|c| c.traditional = on),
            "suggest" => feihe_core::config::update(|c| c.suggest = on),
            "learn" => feihe_core::config::update(|c| c.learn = on),
            // 走 `set_xuanxue` 而不是 `update`：它要同时刷新原子镜像，
            // 而查词热路径读的是镜像（见 `config::xuanxue()`）。
            "xuanxue" => feihe_core::config::set_xuanxue(on),
            _ => {}
        }
    });
}

// ─────────────────────────────────────────────────────────────
// 数据备份：清单 / 说明 / 导入后重载
// ─────────────────────────────────────────────────────────────
//
// **打包解包不在这一层，也不在 core。** core 的依赖政策是「只有 tyme4rs」，zip 要么引
// 库要么手写几百行；而 Kotlin 的 `java.util.zip` 两行就完事。所以边界划在「清单」上：
// core 说**哪些文件算用户数据**，壳负责把它们塞进/取出一个容器。
//
// 这样切的好处：备份的内容只有一份真值，容器格式错了只影响那一次操作，不会让引擎
// 进入「半新半旧」的状态。

/// `NativeEngine.dataFilesNative()` —— 允许备份的数据文件名，`\n` 分隔。
///
/// **导入端必须拿它当白名单**，不能「解压出什么就写什么」：备份包是外部输入，
/// 里面可能有 `../config.ini` 这种路径，也可能有任意大的文件。
#[no_mangle]
pub extern "system" fn Java_com_feihe_ime_engine_NativeEngine_dataFilesNative<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jstring {
    let text = std::panic::catch_unwind(|| feihe_core::backup::DATA_FILES.join("\n"))
        .unwrap_or_default();
    make_string(&mut env, &text)
}

/// `NativeEngine.manifestNative()` —— 导出包里的 `MANIFEST.txt` 正文。
///
/// 带版本号和时间：用户半年后翻出这个文件，至少知道是哪台机器、哪个版本导的。
#[no_mangle]
pub extern "system" fn Java_com_feihe_ime_engine_NativeEngine_manifestNative<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jstring {
    let text = std::panic::catch_unwind(feihe_core::backup::manifest).unwrap_or_default();
    make_string(&mut env, &text)
}

/// `NativeEngine.reloadAllNative()` —— 导入之后把内存态全部重读。
///
/// 必须调：`config` / 词频 / 置顶 / 自造词 / 联想 / 降权 / 自定义短语在各自模块里都
/// 有缓存，不重载的话「导入成功」之后的第一次按键还是旧数据。
#[no_mangle]
pub extern "system" fn Java_com_feihe_ime_engine_NativeEngine_reloadAllNative<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
) {
    // 重载里会读词库、编译 user.bin，耗时在百毫秒量级；panic 不能穿出去（会 abort
    // 整个输入法进程）。
    let _ = std::panic::catch_unwind(feihe_core::backup::reload_all);
}

// 后加的入口分文件放：lib.rs 已经 340 行，再往里塞会让「同一个功能的两端」
// 越离越远。子模块只用到父模块里的 make_string / read_string。
mod dict_extra;
mod quick_extra;
mod sogou_extra;
