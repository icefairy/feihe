//! 前台窗口看门狗：前台窗口/进程一变，立刻通知候选窗收摊。
//!
//! 为什么不能用 `WM_ACTIVATEAPP`：它只投递给**被激活 / 被取消激活的那个进程**
//! 的顶层窗口。我们（中文 TSF DLL）在宿主进程里，切到别的程序时
//! 「本进程」根本没有被激活或取消激活——被取消激活的是宿主进程，消息投给宿主，
//! 我们的候选窗一条都收不到。实测（0.3.19）就是这样：切到浏览器，联想窗照样
//! 挂在新窗口前面。
//!
//! 也不挂 `WH_CALLWNDPROC` / `SetWindowsHookEx`：候选窗建在宿主（记事本、浏览器）
//! 的 TSF 线程上，那条线程我们插不进钩子（既不是我们建的线程，也没法往别人的
//! 消息循环里塞钩子——`SetWindowsHookEx` 只能钩自己进程里别的线程，而且要求
//! 钩子是 DLL 导出、装法完全不同）。
//!
//! 所以改成**轮询**：一个小线程每 [`POLL_MS`] 毫秒看一眼前台窗口句柄，
//! 和上一次记下的不一样就触发回调。开销是一次 `GetForegroundWindow()` +
//! 两个 `Cell` 比较，几十微秒，可以接受。
//!
//! 只看顶层的「前台窗口变了」——不比对进程号，也不关心是不是我们自己：
//! 候选窗是 `WS_EX_NOACTIVATE` 的，自己永远不会成为前台窗口，所以只要前台
//! 换了人，就一定是「跑到了别的窗口」。窗口内换输入框（同程序切两个编辑框）
//! 前台句柄不变，这条看门狗管不着——那要靠 `OnSetFocus`。

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};

use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

/// 轮询间隔。人眼能容忍的「挂一会儿」是几百毫秒；再快就是纯浪费 CPU
/// （TSF 线程本身可能在打字，别和它抢）。
pub const POLL_MS: u64 = 120;

type ChangeFn = Arc<dyn Fn() + Send + Sync>;

static HANDLER: Mutex<Option<ChangeFn>> = Mutex::new(None);
static RUNNING: AtomicBool = AtomicBool::new(false);
/// 上一次看到的前台窗口句柄（0 = 还没记过）
static LAST: AtomicIsize = AtomicIsize::new(0);

/// 注册「前台窗口变了」的回调，并按需拉起看门狗线程。
///
/// 重复调用只换回调，不会起第二个线程。回调在**看门狗线程**上跑，
/// 不在宿主 UI 线程——里面要做的事必须能跨线程（`hide_candidates` 之类
/// 只是 `ShowWindow` + set 几个 `Cell`，安全；**不要**在里面碰 `ITfContext`）。
pub fn set_handler(f: ChangeFn) {
    if let Ok(mut slot) = HANDLER.lock() {
        *slot = Some(f);
    }
    start();
}

/// 撤掉回调（文本服务析构 / 反激活时）。线程会自然停下。
pub fn clear_handler() {
    if let Ok(mut slot) = HANDLER.lock() {
        *slot = None;
    }
}

fn start() {
    if RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("feihe-fg-watch".into())
        .spawn(|| {
            // 起点先记当前前台：否则第一次轮询会把「本来就该出现的窗口」
            // 当成「刚换的」,白收一次（表现为第一次打字时候选闪一下）。
            LAST.store(foreground(), Ordering::Release);
            loop {
                std::thread::sleep(std::time::Duration::from_millis(POLL_MS));

                // 回调被撤了：线程退出，下次 `set_handler` 会重新拉一个
                let cb = {
                    let slot = match HANDLER.lock() {
                        Ok(g) => g,
                        Err(e) => e.into_inner(),
                    };
                    match slot.as_ref() {
                        Some(f) => f.clone(),
                        None => {
                            RUNNING.store(false, Ordering::Release);
                            return;
                        }
                    }
                };

                let now = foreground();
                let prev = LAST.swap(now, Ordering::AcqRel);
                if now != prev {
                    cb();
                }
            }
        });
    if spawned.is_err() {
        RUNNING.store(false, Ordering::Release);
    }
}

fn foreground() -> isize {
    unsafe { GetForegroundWindow().0 as isize }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 前台句柄变了才该触发回调；连着两次读到同一个句柄不该触发。
    ///
    /// 这里直接验判定逻辑——真线程要看桌面会话，单测进程里未必有（CI / 无交互
    /// 会话下 `GetForegroundWindow` 恒为 0），钉住它反而会随机红。
    #[test]
    fn only_fires_when_foreground_changes() {
        let fired = Arc::new(AtomicIsize::new(0));
        let f = fired.clone();
        let decide = move |prev: isize, now: isize| {
            if now != prev {
                f.fetch_add(1, Ordering::AcqRel);
            }
        };
        decide(10, 10);
        assert_eq!(fired.load(Ordering::Acquire), 0, "前台没变不该触发");
        decide(10, 20);
        assert_eq!(fired.load(Ordering::Acquire), 1, "前台变了要触发");
    }

    /// 回调被撤掉之后线程要能自己退出去，`RUNNING` 归位，下次还能重拉。
    #[test]
    fn thread_stops_after_clear() {
        set_handler(Arc::new(|| {}));
        assert!(RUNNING.load(Ordering::Acquire), "set_handler 之后线程该在跑");
        clear_handler();
        // 一个轮询周期之后就该退出；给两倍余量
        std::thread::sleep(std::time::Duration::from_millis(POLL_MS * 2 + 50));
        assert!(
            !RUNNING.load(Ordering::Acquire),
            "回调撤掉后看门狗线程必须停下来（否则每次激活漏一个线程）"
        );
    }
}
