// 飞鹤输入法 · COM 服务计数
// 用于 DllCanUnloadNow：只要有活对象或服务器锁，就不能卸载 DLL。
// 否则 MSCTF 仍持有指向本 DLL 的指针时 DLL 被卸载 -> use-after-unload -> 宿主进程 AV 崩溃。

use std::sync::atomic::{AtomicIsize, Ordering};

static COUNT: AtomicIsize = AtomicIsize::new(0);

/// 新建一个 COM 对象（类工厂 / 文本服务）
pub fn object_created() {
    COUNT.fetch_add(1, Ordering::Relaxed);
}

/// 销毁一个 COM 对象
pub fn object_destroyed() {
    // 防下溢
    let _ = COUNT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |c| {
        Some(if c > 0 { c - 1 } else { 0 })
    });
}

/// LockServer(TRUE/FALSE)
pub fn lock_server(lock: bool) {
    if lock {
        COUNT.fetch_add(1, Ordering::Relaxed);
    } else {
        object_destroyed();
    }
}

/// 是否可以安全卸载 DLL
pub fn can_unload() -> bool {
    COUNT.load(Ordering::Relaxed) <= 0
}
