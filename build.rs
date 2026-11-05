// 构建脚本：用 windres 编译 installer/feihe.rc -> .o，作为链接参数把图标塞进 DLL。
//
// 词库二进制的生成（`base.bin`）搬到了 `core/build.rs`——词库属于 `feihe-core`，
// `include_bytes!(env!("OUT_DIR"))` 必须在 core 自己的 OUT_DIR 里找到它。
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());

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
