// 构建脚本，做两件事：
//
// 1. 生成词库二进制 `$OUT_DIR/base.bin`（见下方 `gen_dict_bin`）
// 2. 用 windres 编译 installer/feihe.rc -> .o，作为链接参数把图标塞进 DLL
use std::path::PathBuf;
use std::process::Command;

// 直接复用运行时的编码器，避免双拼逻辑写两份、改一处忘一处。
// `src/shuangpin.rs` / `src/dictenc.rs` 只依赖 std，可以原样拉进 build script 编译。
#[allow(dead_code)]
#[path = "src/shuangpin.rs"]
mod shuangpin;
#[allow(dead_code)]
#[path = "src/dictenc.rs"]
mod dictenc;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());

    gen_dict_bin(&manifest, &out);

    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() != "windows" {
        return;
    }

    let inc = manifest.join("installer");
    let obj = out.join("feihe-res.o");

    println!("cargo:rerun-if-changed=installer/feihe.rc");
    println!("cargo:rerun-if-changed=installer/resource.h");
    println!("cargo:rerun-if-changed=installer/assets/feihe.ico");

    let rc = inc.join("feihe.rc");
    let inc_s = inc.to_string_lossy().to_string();
    let obj_s = obj.to_string_lossy().to_string();

    // 优先 WINDRES 环境变量；否则按目标架构选 llvm-mingw 的 windres，再退回通用 windres。
    //
    // 注意：windres 产出的 COFF 目标文件带机器类型，x86_64 的 .o 塞进 i686 链接会报
    // `machine type x64 conflicts with x86`，所以这里必须跟着 target 走，
    // 不能像早期版本那样写死 x86_64。
    let triple = match std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default().as_str() {
        "x86" => "i686-w64-mingw32",
        "aarch64" => "aarch64-w64-mingw32",
        _ => "x86_64-w64-mingw32",
    };
    let mut candidates = Vec::new();
    if let Ok(v) = std::env::var("WINDRES") {
        candidates.push(v);
    }
    candidates.push(format!("{triple}-windres"));
    candidates.push("windres".into());

    for wr in &candidates {
        let ok = Command::new(wr)
            .current_dir(&inc)
            .arg(&rc)
            .arg("-I").arg(&inc_s)
            .arg("-O").arg("coff")
            .arg("-o").arg(&obj_s)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            println!("cargo:rustc-link-arg={}", obj_s);
            return;
        }
    }
    println!("cargo:warning=windres 未找到或编译失败，跳过图标资源（IconFile 将无图标）");
}

// ─────────────────────────────────────────────────────────────
// 词库二进制生成
// ─────────────────────────────────────────────────────────────

/// 把 `src/user_dict.txt` + `src/dict.txt` 编译成扁平二进制。
///
/// 为什么不用 `include_str!` + 运行时编码：
/// - 运行时要把 8 万条全拼音节编码成双拼码并排序，`Activate` 里要付 81ms，**每个进程各付一次**
/// - 编码后是堆上的一堆 `Arc`/`Box`，约 9.2MB **私有**内存，同样是每个进程各一份
///
/// 预编码之后，三套码都在构建期算好，运行时只是「读两个 u32 数组 + 二分」，
/// 而 `include_bytes!` 的数据落在 DLL 的 `.rdata` 段里——这一段由 PE 加载器 mmap，
/// **多个进程共享同一份物理页**，实际触达的页数只跟查询次数有关。
fn gen_dict_bin(manifest: &std::path::Path, out: &std::path::Path) {
    // 优先级：用户词在前（同码时排在前面，实现「用户词优先」），
    // 然后是基础词库，最后是人工补充的常用词（只含前两份里缺的词）。
    //
    // `FEIHE_NO_USER_DICT=1` 时跳过 user_dict.txt。
    // 打包分发必须用它：那份文件是从本机搜狗导出的个人打字记录，
    // 一旦并进 base.bin 就随 DLL 一起发出去了，而且是从二进制里抠不出来的。
    let skip_user = std::env::var("FEIHE_NO_USER_DICT").ok().as_deref() == Some("1");
    println!("cargo:rerun-if-env-changed=FEIHE_NO_USER_DICT");

    let mut sources: Vec<&str> = Vec::new();
    if skip_user {
        println!("cargo:warning=FEIHE_NO_USER_DICT=1：已排除 src/user_dict.txt（发布构建）");
    } else {
        sources.push("src/user_dict.txt");
    }
    sources.push("src/dict.txt");
    sources.push("src/extra_dict.txt");

    let mut entries: Vec<(String, String)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for f in sources {
        println!("cargo:rerun-if-changed={f}");
        let Ok(text) = std::fs::read_to_string(manifest.join(f)) else {
            continue;
        };
        dictenc::push_entries(&text, &mut seen, &mut entries);
    }
    if entries.is_empty() {
        println!("cargo:warning=词库为空，base.bin 只会有头部");
    }

    // 编码逻辑与运行时生成 user.bin 共用 `src/dictenc.rs`，不在这里重写一遍。
    let built = dictenc::build(&entries);

    let path = out.join("base.bin");
    std::fs::write(&path, &built.bytes).expect("写 base.bin 失败");
    println!(
        "cargo:warning=base.bin: {} 条, {} KB (小鹤 {} / 微软 {} 可编码)",
        built.count,
        built.bytes.len() / 1024,
        built.xh,
        built.ms
    );
}
