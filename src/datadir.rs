//! Windows 侧的**数据目录策略**：安装目录优先、不可写回落 `%APPDATA%\Feihe`。
//!
//! 从 `config` 里拆出来的。`config` 已经搬进 `feihe-core`，而这段全是 Windows
//! 系统调用（问 DLL 路径、试写文件），留在 core 里会让整个 crate 编不过 Android。
//!
//! 分界线是这样的：**「数据放哪个目录」是壳的事**（Windows 认安装目录、Android 认
//! `Context.getFilesDir()`），**「数据怎么写」是 core 的事**。所以这里只算一个
//! `PathBuf` 出来，交给 `feihe_core::paths`。
//!
//! ## 为什么不在 `DllMain` 里算
//!
//! 这个函数会做文件 I/O（写探针、首次迁移），而 `DllMain` 里碰文件系统会撞
//! loader lock——轻则卡住建 DLL 的线程，重则死锁。所以 `lib.rs` 只是
//! `paths::set_resolver(datadir::resolve)` 交一个函数指针，真正执行发生在第一次
//! 要读写数据文件的时候。

use std::path::{Path, PathBuf};

use windows::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};

/// 旧位置：`%APPDATA%\Feihe`。
pub fn legacy_dir() -> PathBuf {
    let base = std::env::var("APPDATA").unwrap_or_else(|_| "C:\\".to_string());
    PathBuf::from(base).join("Feihe")
}

/// 决定数据目录：优先「DLL 所在目录」，不可写就回落 `%APPDATA%\Feihe`。
///
/// 交给 `feihe_core::paths::set_resolver` 注册。
pub fn resolve() -> PathBuf {
    let fallback = legacy_dir();
    // 测试进程按老规矩走 APPDATA：否则会读到 target\debug 下的文件，而且
    // 「首次运行迁移」会把用户真实的配置拷进构建目录。
    if cfg!(test) {
        return fallback;
    }
    let Some(exe_dir) = module_dir() else {
        return fallback;
    };
    if !writable(&exe_dir) {
        return fallback;
    }
    migrate(&fallback, &exe_dir);
    exe_dir
}

/// 本 DLL 所在目录（不是宿主 EXE 的目录）。
fn module_dir() -> Option<PathBuf> {
    unsafe {
        let mut m = windows::Win32::Foundation::HMODULE::default();
        let addr = module_dir as *const core::ffi::c_void;
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            windows::core::PCWSTR(addr as *const u16),
            &mut m,
        )
        .ok()?;
        let mut buf = [0u16; 1024];
        let n = GetModuleFileNameW(Some(m), &mut buf) as usize;
        if n == 0 || n >= buf.len() {
            return None;
        }
        PathBuf::from(String::from_utf16_lossy(&buf[..n]))
            .parent()
            .map(Path::to_path_buf)
    }
}

/// 目录能不能写。真写一个文件再删掉——查 ACL 不可靠（`Program Files` 的实际
/// 权限取决于组策略，`readonly` 属性也不代表 ACL）。
fn writable(d: &Path) -> bool {
    if !d.is_dir() {
        return false;
    }
    let probe = d.join(".write-probe");
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// 老位置 → 新位置的一次性搬移（复制，不删原件）。
///
/// 只在**新位置还没有 config.ini** 时动手：已经有配置说明用户已经在新位置跑过，
/// 再拷一次会把后来的改动冲掉。用复制不用移动——搬到一半失败（磁盘满、权限）
/// 时老数据还在，用户不会两头落空。
fn migrate(from: &Path, to: &Path) {
    if to.join("config.ini").exists() || from == to {
        return;
    }
    let _ = std::fs::create_dir_all(to);
    for name in ["config.ini", "freq.txt", "phrases.txt", "pinned.txt"] {
        let src = from.join(name);
        let dst = to.join(name);
        if src.exists() && !dst.exists() {
            let _ = std::fs::copy(&src, &dst);
        }
    }
}
