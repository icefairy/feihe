//! 前台窗口看门狗：前台窗口一变，**通知 UI 线程**把候选窗/联想窗收掉。
//!
//! 为什么不能用 `WM_ACTIVATEAPP`：它只投递给**被激活 / 被取消激活的那个进程**
//! 的顶层窗口。我们（中文 TSF DLL）在宿主进程里，切到别的程序时
//! 「本进程」根本没有被激活或取消激活——被取消激活的是宿主进程，消息投给宿主，
//! 我们的候选窗一条都收不到。实测（0.3.19）就是这样：切到浏览器，联想窗照样
//! 挂在新窗口前面。
//!
//! 也不挂 `WH_CALLWNDPROC` / `SetWindowsHookEx`：候选窗建在宿主（记事本、浏览器）
//! 的 TSF 线程上，那条线程我们插不进钩子。
//!
//! 所以改成**轮询**：一个小线程每 [`POLL_MS`] 毫秒看一眼前台窗口句柄，
//! 变了就置一个 [`AtomicBool`] 脏标记，并给候选窗投一条 `WM_APP` 消息
//! （`crate::candidate::post_hide_for_foreground_change`）。
//!
//! ## 0.3.20 → 0.3.21 的教训：回调**绝不跨线程**
//!
//! 0.3.20 的看门狗线程直接调回调，回调里干了两件在别的线程上不能干的事：
//!
//! 1. `candidate::hide_for_foreground_change()` —— 在**非 UI 线程** `ShowWindow`
//!    候选窗；
//! 2. `&*(addr as *const FeiheTextService_Impl)` 然后 `this.fg_dirty.set(true)`
//!    —— 从另一个线程写一个 `Cell<bool>`（`FeiheTextService_Impl` 整个是 `!Sync`
//!    的）。按 Rust 的内存模型这是 UB；实际表现是**点浮窗「设置」就崩**：
//!    设置窗 `SetForegroundWindow` 让前台一变，看门狗线程立刻进来，和 UI 线程里
//!    正在 `CreateWindowExW` 建设置窗的过程撞上。
//!
//! 现在线程**不再碰文本服务、不再碰窗口**，只做两件跨线程安全的事：
//! - `DIRTY.store(true)`（ATOMIC，本来就可以跨线程）；
//! - `PostMessageW` 一条消息（Win32 明确允许跨线程投递）。
//!
//! 真正的收窗在候选窗自己的窗口过程（UI 线程）里做；文本服务的状态由 UI 线程
//! 在 `OnSetFocus` / 按键里通过 [`take_dirty`] 抢清。见 `candidate` 与
//! `textservice::flush_foreground_change`。
//!
//! 只看顶层的「前台窗口变了」——不比对进程号，也不关心是不是我们自己：
//! 候选窗是 `WS_EX_NOACTIVATE` 的，自己永远不会成为前台窗口，所以只要前台
//! 换了人，就一定是「跑到了别的窗口」。窗口内换输入框（同程序切两个编辑框）
//! 前台句柄不变，这条看门狗管不着——那要靠 `OnSetFocus`。

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

/// 轮询间隔。人眼能容忍的「挂一会儿」是几百毫秒；再快就是纯浪费 CPU
/// （TSF 线程本身可能在打字，别和它抢）。
pub const POLL_MS: u64 = 120;

/// 看门狗线程是否在跑。
static RUNNING: AtomicBool = AtomicBool::new(false);
/// 上一次看到的前台窗口句柄（0 = 还没记过）。
static LAST: AtomicIsize = AtomicIsize::new(0);
/// 「前台换人了」的待处理脏标记：看门狗线程置、UI 线程抢清（见 [`take_dirty`]）。
///
/// 用 `AtomicBool` 而不是文本服务里的 `Cell<bool>`：这是唯一从看门狗线程
/// 写出去的状态，必须是能在任意线程安全存取的原子量。
static DIRTY: AtomicBool = AtomicBool::new(false);

/// 让看门狗线程能被立刻唤醒退出（`wait_timeout` 的谓词）。
static WAKE: Mutex<bool> = Mutex::new(false);
static WAKE_CV: Condvar = Condvar::new();
/// 看门狗线程句柄，`disable` 里 join 掉，保证返回后不会再有线程跑本 DLL 的代码
/// ——否则 `Disable` 之后到 DLL 卸载之间那个 ≤120ms 的窗口里，线程正好醒来
/// 执行，代码页已被卸载就是一次 use-after-unload 崩溃。
static THREAD: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

/// 拉起看门狗线程（重复调用幂等）。
///
/// 线程只做「发现前台变了 → 置脏 + 投消息」，不碰文本服务、不碰窗口。
pub fn enable() {
    // 每次激活从「干净」开始：上一次残留的脏标记不该影响这一次
    DIRTY.store(false, Ordering::Release);
    if RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    if let Ok(mut g) = WAKE.lock() {
        *g = false;
    }

    let spawned = std::thread::Builder::new()
        .name("feihe-fg-watch".into())
        .spawn(|| {
            // 起点先记当前前台：否则第一次轮询会把「本来就该出现的窗口」
            // 当成「刚换的」，白收一次（表现为第一次打字时候选闪一下）。
            LAST.store(foreground(), Ordering::Release);
            loop {
                // 睡一个周期，但 `disable` 能立刻唤醒（见 WAKE_CV）
                {
                    let guard = match WAKE.lock() {
                        Ok(g) => g,
                        Err(e) => e.into_inner(),
                    };
                    let (guard, _) = match WAKE_CV.wait_timeout(guard, Duration::from_millis(POLL_MS))
                    {
                        Ok(v) => v,
                        Err(e) => e.into_inner(),
                    };
                    if *guard {
                        break;
                    }
                }
                if !RUNNING.load(Ordering::Acquire) {
                    break;
                }

                let now = foreground();
                let prev = LAST.swap(now, Ordering::AcqRel);
                if now != prev {
                    // 跨线程安全的两个动作：原子标记 + 投消息。真正收窗在 UI 线程。
                    DIRTY.store(true, Ordering::Release);
                    crate::candidate::post_hide_for_foreground_change();
                }
            }
        });

    match spawned {
        Ok(h) => {
            if let Ok(mut slot) = THREAD.lock() {
                *slot = Some(h);
            }
        }
        Err(_) => {
            RUNNING.store(false, Ordering::Release);
        }
    }
}

/// 停掉看门狗线程（反激活 / 析构时）。返回时线程已经退出。
pub fn disable() {
    RUNNING.store(false, Ordering::Release);
    if let Ok(mut g) = WAKE.lock() {
        *g = true;
    }
    WAKE_CV.notify_all();
    let handle = THREAD.lock().ok().and_then(|mut s| s.take());
    if let Some(h) = handle {
        let _ = h.join();
    }
}

/// 取走「前台换过人」的脏标记（UI 线程用）。返回 true 表示需要清一次联想/候选状态。
pub fn take_dirty() -> bool {
    DIRTY.swap(false, Ordering::AcqRel)
}

fn foreground() -> isize {
    unsafe { GetForegroundWindow().0 as isize }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicIsize;
    use std::sync::{Arc, Mutex as StdMutex, MutexGuard};

    /// 碰全局量（`RUNNING` / `DIRTY` / `THREAD`）的用例共用一把锁。
    ///
    /// `cargo test` 默认多线程，`enable()` 会把 `DIRTY` 清零，正好撞上
    /// `take_dirty_is_one_shot` 的两次读——串起来跑才稳定。
    fn seq() -> MutexGuard<'static, ()> {
        static LOCK: StdMutex<()> = StdMutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 前台句柄变了才算「换人」；连着两次读到同一个句柄不该算。
    ///
    /// 这里只验判定逻辑——真线程要看桌面会话，单测进程里未必有（CI / 无交互
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

    /// `disable` 之后线程必须已经退出、`RUNNING` 归位，而且下次还能重拉。
    ///
    /// 0.3.20 只把 `RUNNING` 置 false，线程要等下一次轮询才退出；本版 `disable`
    /// 用条件变量唤醒并 `join`，返回即「线程已死」——这是 DLL 卸载安全的前提。
    #[test]
    fn disable_joins_thread_and_can_reenable() {
        let _g = seq();
        enable();
        assert!(RUNNING.load(Ordering::Acquire), "enable 之后线程该在跑");
        disable();
        assert!(
            !RUNNING.load(Ordering::Acquire),
            "disable 之后 RUNNING 必须归位"
        );
        assert!(
            THREAD.lock().map(|s| s.is_none()).unwrap_or(true),
            "disable 之后线程句柄必须被取走（已 join）"
        );
        // 还能再拉起来（激活→反激活→再激活 是常态）
        enable();
        assert!(RUNNING.load(Ordering::Acquire), "重新 enable 线程该在跑");
        disable();
    }

    /// `take_dirty` 是一次性的：取过一次就清，第二次读到 false。
    #[test]
    fn take_dirty_is_one_shot() {
        let _g = seq();
        DIRTY.store(true, Ordering::Release);
        assert!(take_dirty(), "置脏之后第一次应读到 true");
        assert!(!take_dirty(), "脏标记取过一次就该清掉");
    }
}
