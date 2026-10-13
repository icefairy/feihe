// 构建脚本：把图标资源编译进 feihe.dll
// 用 windres 编译 installer/feihe.rc -> .o，再作为链接参数传入
use std::path::PathBuf;
use std::process::Command;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() != "windows" {
        return;
    }

    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let inc = manifest.join("installer");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let obj = out.join("feihe-res.o");

    println!("cargo:rerun-if-changed=installer/feihe.rc");
    println!("cargo:rerun-if-changed=installer/resource.h");
    println!("cargo:rerun-if-changed=installer/assets/feihe.ico");

    let rc = inc.join("feihe.rc");
    let inc_s = inc.to_string_lossy().to_string();
    let obj_s = obj.to_string_lossy().to_string();

    // 优先 WINDRES 环境变量；否则试 x86_64-w64-mingw32-windres（llvm-mingw），再退回 windres
    let mut candidates = Vec::new();
    if let Ok(v) = std::env::var("WINDRES") {
        candidates.push(v);
    }
    candidates.push("x86_64-w64-mingw32-windres".into());
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
