//! 飞鹤输入法 · **最小 TSF 宿主**（UILess 测试床）
//!
//! 这个程序不是编辑器，只做一件事：**扮演一个 UI-less 宿主**，验证飞鹤的
//! `ITfUIElementSink` 那一半到底通不通。
//!
//! ## 怎么用
//!
//! ```text
//! cargo run --release                     # UILess 模式（宿主要求自绘候选）
//! cargo run --release -- --normal         # 对照组：让 TIP 自己弹候选窗
//! cargo run --release -- --switch         # 先把当前输入法切到飞鹤再激活
//! ```
//!
//! 跑起来后点进窗口，打 `nihao` 之类。判据很直接：
//!
//! | 现象 | 结论 |
//! | --- | --- |
//! | 窗口里出现候选条，`Begin` 计数在涨 | UILess 通了，候选是宿主画的 |
//! | 没候选条，但屏幕角落弹了飞鹤自己的候选框 | 走了自绘窗，UILess 没生效 |
//! | 连候选都没有、打不出汉字 | TIP 没加载，或中英文态在英文 |
//!
//! ## 为什么这么写
//!
//! 一个 TSF 宿主最少要有四样东西，缺一样 TIP 就罢工：
//!
//! 1. **激活的线程管理器**：`ITfThreadMgrEx::ActivateEx(&tid, TF_AE_UIELEMENTENABLEDONLY)`。
//!    标志位是 UILess 的**唯一**开关，MSCTF 会把它原样透传给 TIP 的 `ActivateEx`。
//! 2. **文档管理器 + 上下文**：`CreateDocumentMgr` → `CreateContext(ITextStoreACP)`
//!    → `Push` → `SetFocus`。没有上下文，TIP 连 `RequestEditSession` 都没地方发，
//!    表现就是「按键有反应、但一个字都上不去」。
//! 3. **`ITextStoreACP`**：见 `store.rs`。
//! 4. **`ITfUIElementSink`**：见 `uisink.rs`，里面那一行 `*pbshow = FALSE` 就是
//!    「宿主自己画候选」的全部秘密。
//!
//! 按键必须**主动转发**：TSF 不会自己截键，宿主收到 `WM_KEYDOWN` 要依次调
//! `ITfKeystrokeMgr::TestKeyDown`（问「这个键你要不要」）和 `KeyDown`（真派发），
//! 两步都为真才吞掉这个键。真实游戏、WPF、DirectWrite 应用都是这么做的。

mod state;
mod store;
mod uisink;

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Mutex;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    keybd_event, GetKeyboardLayout, GetKeyState, MapVirtualKeyW, SetFocus, HKL, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, VK_CONTROL, VK_Q, VK_SHIFT,
};
use windows::Win32::UI::TextServices::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use store::{Store, LINE_H, TEXT_LEFT, TEXT_TOP};

/// 和 `src/clsid.rs` 保持一致（宿主是个独立 crate，不能 `use` 主库）
const CLSID_FEIHE_TEXTSERVICE: GUID = GUID::from_u128(0x70cb823d_d08a_4b57_b8be_08bce2539ff3);
const GUID_PROFILE_FEIHE: GUID = GUID::from_u128(0xda6b9170_1db7_495f_8741_d2c6f3b5e28f);
const LANGID_CHS: u16 = 0x0804;

/// 宿主侧的 `TF_TMAE_UIELEMENTENABLEDONLY`（msctf.h）= **4**。
///
/// 注意别写成 `TF_AE_UIELEMENTENABLEDONLY`（=1）——那是 TIP 侧
/// `ITfTextInputProcessorEx::ActivateEx` 的 `TfActiveFlags`，两套枚举、两套数值。
/// 宿主要是传 1，传的实际是 `TF_TMAE_NOACTIVATETIP`：TIP 会被推迟到线程获得焦点时
/// 才异步激活，flags 直接丢掉，UILess 永远不生效，而且**不报错**。
const TF_TMAE_UIELEMENTENABLEDONLY: u32 = 0x0000_0004;

/// 一次运行期间需要保活的 TSF 对象
struct Host {
    tim: ITfThreadMgr,
    doc: ITfDocumentMgr,
    source: ITfSource,
    cookie: u32,
}

/// 自测退出码：0 通过，2 没通
static EXIT_CODE: AtomicI32 = AtomicI32::new(0);

/// 自测敲的键。用「李明远」的编码——顺带把自造词那条路也过一遍。
const SELFTEST_KEYS: &str = "limkyr";

/// Shift+字母自测：中文态按 Shift+A，文档里应该只有一个大写 `A`。
const SHIFT_TEST_KEYS: &str = "A";

/// 自测预期上屏的文本。空 = 「只要求收到候选」，非空 = 「文档必须等于它」。
static EXPECT_DOC: Mutex<Option<String>> = Mutex::new(None);

thread_local! {
    static HOST: RefCell<Option<Host>> = RefCell::new(None);
    /// 上一个 `WM_KEYDOWN` 被 TIP 吃了，紧随其后的 `WM_CHAR` 也要丢掉。
    static SWALLOW_CHAR: Cell<bool> = Cell::new(false);
}

fn main() {
    if let Err(e) = run() {
        eprintln!("启动失败：{e}");
        std::process::exit(1);
    }
}

fn print_help() {
    println!(
        "飞鹤 · 最小 TSF 宿主

用法：
  tsfhost                  UILess 模式（ActivateEx 带 TF_TMAE_UIELEMENTENABLEDONLY=4）
  tsfhost --normal         对照组，flags = 0，TIP 会自己弹候选窗
  tsfhost --switch         先把当前输入法切到飞鹤再激活（会改变系统输入法状态）
  tsfhost --manual         不经语言栏，自己 CoCreate 飞鹤 TIP 并 ActivateEx(1)
  tsfhost --register       给飞鹤补注册 UIELEMENTENABLED 分类（需管理员）
  tsfhost --selftest       自动敲「limkyr」，打印结果后退出（退出码 0=PASS 2=FAIL）
  tsfhost --selftest-shift 自动敲 Shift+A，文档里应只有一个大写 A
  tsfhost --help

窗口里 Ctrl+Q 退出。

--selftest 适合命令行一把跑完，只看 BeginUIElement 有没有被调用；
不带参数是给人手动试的：能看见候选条、打字能上屏，才算真通。"
    );
}

fn run() -> Result<()> {
    let mut flags = TF_TMAE_UIELEMENTENABLEDONLY;
    let mut switch = false;
    let mut selftest = false;
    let mut shift_test = false;
    let mut manual = false;
    let mut register = false;
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--switch" => switch = true,
            "--normal" => flags = 0,
            "--manual" => manual = true,
            "--register" => register = true,
            "--selftest-shift" => {
                shift_test = true;
                selftest = true;
                switch = true;
            }
            "--selftest" => {
                selftest = true;
                switch = true; // 自测必须保证确实在用飞鹤，否则测的是别的输入法
            }
            "--help" | "-h" => {
                print_help();
                return Ok(());
            }
            other => eprintln!("忽略未知参数：{other}"),
        }
    }

    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;

        if register {
            register_category()?;
            println!("[host] 已注册 GUID_TFCAT_TIPCAP_UIELEMENTENABLED");
            return Ok(());
        }

        // ① 建管理器 → 建窗口 → 切输入法 → 激活。每一步的理由见 create_thread_mgr。
        let tim = create_thread_mgr()?;

        // ① 先 ActivateEx 把宿主的意图（flags）记到线程管理器上。
        //    此时 TIP 通常还没被拉起来，真正的加载发生在地 ② 步——所以 TIP 那侧
        //    不能只看 ActivateEx 的 dwflags，还要 `GetActiveFlags` 兜底。
        let tid = activate_thread(&tim, flags)?;

        // ② 建窗口并给焦点。user32 的 IMM32 兼容层会在这里把当前输入法激活一遍。
        let hwnd = create_window()?;
        eprintln!("[host] window created hkl=0x{:X}", hkl());
        let _ = SetFocus(Some(hwnd));
        let _ = SetForegroundWindow(hwnd);

        // ③ 切到飞鹤（FORPROCESS 只影响本进程）。这一步会顺带把飞鹤 TIP 拉起来。
        if switch {
            switch_to_feihe();
        }
        eprintln!("[host] after switch, feihe loaded={}", tip_loaded());

        attach_document(hwnd, tim, tid, manual)?;

        if selftest {
            if shift_test {
                *EXPECT_DOC.lock().unwrap() = Some(SHIFT_TEST_KEYS.to_string());
            }
            let _ = SetForegroundWindow(hwnd);
            // 稍等一下让窗口真正前台，再开始注入按键
            let id = SetTimer(Some(hwnd), 1, 700, None);
            eprintln!("[host] selftest timer id={id}");
        }

        pump();
        shutdown();

        let _ = DestroyWindow(hwnd);
        CoUninitialize();
    }

    match EXIT_CODE.load(Ordering::Relaxed) {
        0 => Ok(()),
        n => std::process::exit(n),
    }
}

// ─────────────────────────────────────────────────────────────
// TSF 初始化
// ─────────────────────────────────────────────────────────────

/// 拿到本线程的 TSF 管理器（**不激活**）。
///
/// 激活会被推迟到窗口建好之后，因为激活的一瞬间 MSCTF 才会去加载 TIP，
/// 而加载哪个 TIP 由当前键盘布局（HKL）决定。顺序因此在 `run` 里被拆成：
///
/// 建管理器 → 建窗口 → 切输入法 → ActivateEx → 建文档
///
/// 其中「切输入法」必须在窗口存在之后：`ActivateLanguageProfile` 要靠前台窗口
/// 才能把 HKL 真正换过去，没有窗口时它返回 S_OK 但什么都没发生——**不报错**。
fn create_thread_mgr() -> Result<ITfThreadMgr> {
    unsafe {
        let tim: ITfThreadMgr =
            CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)?;
        eprintln!("[host] thread mgr ok hkl=0x{:X}", hkl());
        Ok(tim)
    }
}

/// 查 TIP 有没有注册 UIELEMENTENABLED 分类。
///
/// 这是 UILess 能不能用的**前置条件**：宿主以 `TF_TMAE_UIELEMENTENABLEDONLY`
/// 激活 TSF 时，MSCTF 只加载带这个分类的 TIP。没注册的话输入法整个不出现，
/// `ActivateEx` 却照样返回 S_OK——没有任何报错可循，只能靠查分类发现。
fn category_registered() -> bool {
    unsafe {
        let Ok(cat) = CoCreateInstance::<_, ITfCategoryMgr>(
            &CLSID_TF_CategoryMgr,
            None,
            CLSCTX_INPROC_SERVER,
        ) else {
            return false;
        };
        let Ok(e) = cat.EnumItemsInCategory(&GUID_TFCAT_TIPCAP_UIELEMENTENABLED) else {
            return false;
        };
        let mut buf = [GUID::zeroed(); 8];
        loop {
            let mut n = 0u32;
            let _ = e.Next(&mut buf, Some(&mut n));
            if n == 0 {
                return false;
            }
            if buf[..n as usize].contains(&CLSID_FEIHE_TEXTSERVICE) {
                return true;
            }
        }
    }
}

/// 注册分类。要管理员权限——安装脚本本来就会写这一条，这里只是给测试省事。
fn register_category() -> Result<()> {
    unsafe {
        let cat: ITfCategoryMgr =
            CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)?;
        cat.RegisterCategory(
            &CLSID_FEIHE_TEXTSERVICE,
            &GUID_TFCAT_TIPCAP_UIELEMENTENABLED,
            &CLSID_FEIHE_TEXTSERVICE,
        )?;
    }
    Ok(())
}

fn tip_loaded() -> bool {
    unsafe { GetModuleHandleW(w!("feihe.dll")).is_ok() }
}

fn hkl() -> isize {
    unsafe { GetKeyboardLayout(0).0 as isize }
}

fn switch_to_feihe() {
    unsafe {
        match CoCreateInstance::<_, ITfInputProcessorProfiles>(
            &CLSID_TF_InputProcessorProfiles,
            None,
            CLSCTX_INPROC_SERVER,
        ) {
            Ok(p) => {
                let before = hkl();
                // 用 ActivateProfile 而不是 ActivateLanguageProfile：前者会连键盘布局
                // 一起换过去（HKL 从 0x8040804 变成 0xE0xx0804），后者只改语言栏状态，
                // HKL 不变——而 MSCTF 在 ActivateEx 时正是靠 HKL 决定加载哪个 TIP。
                let r = p.cast::<ITfInputProcessorProfileMgr>().and_then(|m| {
                    m.ActivateProfile(
                        TF_PROFILETYPE_INPUTPROCESSOR,
                        LANGID_CHS,
                        &CLSID_FEIHE_TEXTSERVICE,
                        &GUID_PROFILE_FEIHE,
                        HKL::default(),
                        TF_IPPMF_FORPROCESS | TF_IPPMF_DONTCARECURRENTINPUTLANGUAGE,
                    )
                });
                eprintln!(
                    "[host] ActivateProfile -> err={:?} hkl: 0x{before:X} -> 0x{:X}",
                    r.as_ref().err().map(|e| e.code().0),
                    hkl()
                );
                if let Err(e) = r {
                    state::lock().note = format!(
                        "切换输入法失败（0x{:08X}），请手动 Win+Space 切到飞鹤",
                        e.code().0
                    );
                }
            }
            Err(e) => {
                state::lock().note = format!("拿不到输入法配置接口：{e}");
            }
        }
    }
}

/// 激活线程管理器。flags 就是在这里被 MSCTF 记下并转给 TIP 的 ActivateEx 的。
fn activate_thread(tim: &ITfThreadMgr, flags: u32) -> Result<u32> {
    unsafe {
        let timex: ITfThreadMgrEx = tim.cast()?;
        let mut tid = 0u32;
        timex.ActivateEx(&mut tid, flags)?;
        let active = timex.GetActiveFlags().unwrap_or(0);
        eprintln!("[host] ActivateEx tid={tid} flags=0x{flags:X} active=0x{active:X}");
        state::lock().active_flags = active;
        Ok(tid)
    }
}

fn create_window() -> Result<HWND> {
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let class = w!("FeiheTsfHostWindow");
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as isize as *mut core::ffi::c_void),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassW(&wc);

        Ok(CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            w!("飞鹤 · 最小 TSF 宿主（UILess 测试）"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            980,
            580,
            None,
            None,
            Some(hinst.into()),
            None,
        )?)
    }
}

/// 建文档、上下文、注册 UIElement sink，然后把一切都挂进 `HOST`。
fn attach_document(hwnd: HWND, tim: ITfThreadMgr, tid: u32, manual: bool) -> Result<()> {
    let tim = &tim;
    unsafe {
        let doc: ITfDocumentMgr = tim.CreateDocumentMgr()?;
        eprintln!("[host] doc mgr ok");
        let store: ITextStoreACP = Store::new(hwnd).into();
        let unk: IUnknown = store.cast()?;
        let mut ctx_opt: Option<ITfContext> = None;
        let mut _ec = 0u32;
        // 第二个参数（dwFlags）在 tsf.h 的注释里看不到，但 win32metadata 里有，
        // 传 0 即可。别想着去掉它——按 4 个参数自己 transmute 调会直接崩。
        doc.CreateContext(tid, 0, &unk, &mut ctx_opt, &mut _ec)?;
        let ctx = ctx_opt.ok_or_else(|| Error::from(E_FAIL))?;
        eprintln!("[host] context ok ec={_ec}");
        doc.Push(&ctx)?;
        // 让 TSF 知道「焦点在这个文档上」，否则 TIP 的 GetFocus 拿不到上下文
        tim.SetFocus(&doc)?;
        eprintln!("[host] focus ok");

        // 手动模式：自己 CoCreate TIP 再 ActivateEx，绕开语言栏。
        //
        // 这里的 dwflags 是 **TIP 侧** 的 `TfActiveFlags`，值 1；宿主侧那份是
        // `TF_TMAE_UIELEMENTENABLEDONLY`，值 4。两套枚举，最容易搞混的一处。
        //
        // 注意：如果语言栏已经激活了这个 TIP，同一 tid 再挂一次 key sink 会
        // 返回 E_INVALIDARG（0x80070057），这是正常的——说明它已经在跑了。
        if manual {
            match CoCreateInstance::<_, IUnknown>(&CLSID_FEIHE_TEXTSERVICE, None, CLSCTX_INPROC_SERVER) {
                Ok(o) => match o.cast::<ITfTextInputProcessorEx>() {
                    Ok(tip) => {
                        let r = tip.ActivateEx(tim, tid, 1);
                        eprintln!(
                            "[host] manual TIP ActivateEx -> err={:?}",
                            r.as_ref().err().map(|e| e.code().0)
                        );
                    }
                    Err(e) => eprintln!("[host] TIP 不支持 ITfTextInputProcessorEx: {e}"),
                },
                Err(e) => eprintln!("[host] CoCreateInstance(feihe) 失败: {e}"),
            }
        }

        // 注册宿主侧的 UIElement sink —— UILess 另一半就在这里
        let mgr: ITfUIElementMgr = tim.cast()?;
        let sink: ITfUIElementSink = uisink::UISink::new(mgr, hwnd).into();
        let source: ITfSource = tim.cast()?;
        let cookie = source.AdviseSink(
            &<ITfUIElementSink as Interface>::IID,
            &sink.cast::<IUnknown>()?,
        )?;
        eprintln!("[host] uielement sink ok");

        let tip_loaded = tip_loaded();
        let category = category_registered();
        {
            let mut st = state::lock();
            st.tip_loaded = tip_loaded;
            st.category = category;
            if st.note.is_empty() && !category {
                st.note = "飞鹤没注册 UIELEMENTENABLED 分类：UILess 下 MSCTF 不会加载它（改用 --normal 看普通模式，或用管理员跑 tsfhost --register）".into();
            } else if st.note.is_empty() && !tip_loaded {
                st.note = "feihe.dll 未加载：当前输入法不是飞鹤，用 Win+Space 切过来（或加 --switch）".into();
            }
        }
        eprintln!("[host] UIELEMENTENABLED 分类 = {category}");

        println!("[host] tid={tid} active_flags=0x{:X}", state::lock().active_flags);
        println!("[host] feihe.dll loaded = {tip_loaded}");
        println!("[host] 窗口里打拼音试试，Ctrl+Q 退出");

        HOST.with(|h| {
            *h.borrow_mut() = Some(Host {
                tim: tim.clone(),
                doc,
                source,
                cookie,
            })
        });
    }
    Ok(())
}
fn shutdown() {
    HOST.with(|h| {
        if let Some(host) = h.borrow_mut().take() {
            unsafe {
                let _ = host.source.UnadviseSink(host.cookie);
                let _ = host.doc.Pop(0);
                let _ = host.tim.Deactivate();
            }
        }
    });
}

// ─────────────────────────────────────────────────────────────
// 消息循环与按键转发
// ─────────────────────────────────────────────────────────────

fn pump() {
    let mut msg = MSG::default();
    loop {
        let r = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if !r.as_bool() {
            break;
        }
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// 把按键交给 TSF。返回 `true` = TIP 吃掉了这个键，宿主不要再处理。
fn forward_key(wparam: WPARAM, lparam: LPARAM, down: bool) -> bool {
    HOST.with(|h| {
        let b = h.borrow();
        let Some(host) = b.as_ref() else {
            return false;
        };
        let Ok(km) = host.tim.cast::<ITfKeystrokeMgr>() else {
            return false;
        };
        unsafe {
            if down {
                // TestKeyDown 只问「想不想看」，返回 FALSE 表示不感兴趣，键照旧给应用
                if !km
                    .TestKeyDown(wparam, lparam)
                    .map(|b| b.as_bool())
                    .unwrap_or(false)
                {
                    return false;
                }
                km.KeyDown(wparam, lparam)
                    .map(|b| b.as_bool())
                    .unwrap_or(false)
            } else {
                if !km
                    .TestKeyUp(wparam, lparam)
                    .map(|b| b.as_bool())
                    .unwrap_or(false)
                {
                    return false;
                }
                km.KeyUp(wparam, lparam)
                    .map(|b| b.as_bool())
                    .unwrap_or(false)
            }
        }
    })
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if GetKeyState(VK_CONTROL.0 as i32) < 0 && wparam.0 == VK_Q.0 as usize {
                let _ = DestroyWindow(hwnd);
                return LRESULT(0);
            }
            let eaten = forward_key(wparam, lparam, true);
            SWALLOW_CHAR.with(|c| c.set(eaten));
            if eaten {
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
        WM_KEYUP => {
            if forward_key(wparam, lparam, false) {
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
        WM_CHAR => {
            // 被 TIP 吃掉的键不该再产生字符。剩下的一律丢掉——本宿主不处理
            // 「非输入法输入」，只显示 TSF 写进文档的内容。
            SWALLOW_CHAR.with(|c| c.set(false));
            LRESULT(0)
        }
        WM_TIMER => {
            eprintln!("[host] WM_TIMER {}", wparam.0);
            match wparam.0 {
                1 => {
                    let _ = KillTimer(Some(hwnd), 1);
                    if EXPECT_DOC.lock().unwrap().is_some() {
                        inject_keys(SHIFT_TEST_KEYS);
                    } else {
                        inject_keys(SELFTEST_KEYS);
                    }
                    // 每个键约 95ms，6 个键 ~600ms，再留 1.5s 让 TIP 把候选推过来
                    let id2 = SetTimer(Some(hwnd), 2, 1600, None);
                    eprintln!("[host] second timer id={id2}");
                }
                2 => finish_selftest(hwnd),
                _ => {}
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            // 点一下就重新抢焦点，方便从别的窗口切回来
            let _ = SetFocus(Some(hwnd));
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ─────────────────────────────────────────────────────────────
// 自测：自己敲键、自己看结果
// ─────────────────────────────────────────────────────────────

/// 用 `keybd_event` 注入按键。
///
/// 为什么不直接 `PostMessage(WM_KEYDOWN)`：主机路径上 TIP 会读 `GetKeyState` 判断修饰键，
/// 也会依赖真实的扫描码顺序，伪造消息容易测出「假通过」。
fn inject_keys(s: &str) {
    for ch in s.chars() {
        // 大写字母 = 按住 Shift 敲（`keybd_event` 不会自己带修饰键，
        // 必须把 Shift 也注入进去，否则测的是「不按 Shift」的那条路）。
        let shifted = ch.is_ascii_uppercase();
        let c = ch.to_ascii_uppercase();
        if !c.is_ascii_uppercase() {
            continue;
        }
        let vk = c as u16;
        let sc = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) } as u8;
        let shift_sc = unsafe { MapVirtualKeyW(VK_SHIFT.0 as u32, MAPVK_VK_TO_VSC) } as u8;
        unsafe {
            if shifted {
                keybd_event(VK_SHIFT.0 as u8, shift_sc, KEYBD_EVENT_FLAGS(0), 0);
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            keybd_event(vk as u8, sc, KEYBD_EVENT_FLAGS(0), 0);
            std::thread::sleep(std::time::Duration::from_millis(25));
            keybd_event(vk as u8, sc, KEYEVENTF_KEYUP, 0);
            if shifted {
                std::thread::sleep(std::time::Duration::from_millis(20));
                keybd_event(VK_SHIFT.0 as u8, shift_sc, KEYEVENTF_KEYUP, 0);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(70));
    }
}

fn finish_selftest(hwnd: HWND) {
    let (begins, updates, ends, cands, doc, note) = {
        let st = state::lock();
        (
            st.begins,
            st.updates,
            st.ends,
            st.candidates.clone(),
            st.doc.clone(),
            st.note.clone(),
        )
    };

    let expect = EXPECT_DOC.lock().unwrap().clone();
    let ok = match expect.as_deref() {
        // Shift+字母：不要求候选，要求文档里恰好是那个大写字母
        Some(want) => doc == *want,
        None => begins > 0 && !cands.is_empty(),
    };
    println!("\n===== UILess 自测 =====");
    println!("BeginUIElement  = {begins}");
    println!("UpdateUIElement = {updates}");
    println!("EndUIElement    = {ends}");
    println!("文档内容 = {doc:?}");
    println!("候选     = {cands:?}");
    if !note.is_empty() {
        println!("备注     = {note}");
    }
    println!(
        "结论     = {}",
        match (ok, expect.as_deref()) {
            (true, Some(want)) => format!("PASS · Shift+字母 直接上屏了 {want:?}"),
            (false, Some(want)) => format!("FAIL · 期望文档是 {want:?}，实际是 {doc:?}"),
            (true, None) => "PASS · 候选由宿主绘制，UILess 通道正常".to_string(),
            (false, None) =>
                "FAIL · 没收到候选：检查输入法是否切到飞鹤、中英文态是否在中文".to_string(),
        }
    );

    if !ok {
        EXIT_CODE.store(2, Ordering::Relaxed);
    }
    unsafe {
        let _ = KillTimer(Some(hwnd), 1);
        let _ = KillTimer(Some(hwnd), 2);
        let _ = DestroyWindow(hwnd);
    }
}

// ─────────────────────────────────────────────────────────────
// 绘制
// ─────────────────────────────────────────────────────────────

pub fn client_w(hwnd: HWND) -> i32 {
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
    }
    rc.right
}

pub fn client_h(hwnd: HWND) -> i32 {
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
    }
    rc.bottom
}

unsafe fn draw_text(hdc: HDC, s: &str, mut rc: RECT, color: COLORREF, flags: DRAW_TEXT_FORMAT) -> i32 {
    let mut w: Vec<u16> = s.encode_utf16().collect();
    SetTextColor(hdc, color);
    DrawTextW(hdc, &mut w, &mut rc, flags)
}

unsafe fn text_width(hdc: HDC, s: &str) -> i32 {
    let w: Vec<u16> = s.encode_utf16().collect();
    if w.is_empty() {
        return 0;
    }
    let mut size = SIZE::default();
    let _ = GetTextExtentPoint32W(hdc, &w, &mut size);
    size.cx
}

unsafe fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let (cw, ch) = (client_w(hwnd), client_h(hwnd));

    // 背景
    let bg = CreateSolidBrush(COLORREF(0x00FB_FB_FB));
    FillRect(hdc, &RECT { left: 0, top: 0, right: cw, bottom: ch }, bg);
    let _ = DeleteObject(bg.into());

    SetBkMode(hdc, TRANSPARENT);
    let font = GetStockObject(DEFAULT_GUI_FONT);
    let old_font = SelectObject(hdc, font);

    let (status, note, doc, cands, sel, page, pages, active, uil, tip) = {
        let st = state::lock();
        (
            format!(
                "active_flags=0x{:X}  分类={}  TIP={}  Begin={} Update={} End={}  候选 {} 个 · 第 {} 页",
                st.active_flags,
                if st.category { "有" } else { "缺" },
                if st.tip_loaded { "feihe.dll 已加载" } else { "未加载" },
                st.begins,
                st.updates,
                st.ends,
                st.candidates.len(),
                st.cand_page + 1,
            ),
            st.note.clone(),
            st.doc.clone(),
            st.candidates.clone(),
            st.cand_sel,
            st.cand_page,
            st.pages.clone(),
            st.cand_active,
            st.active_flags & TF_TMAE_UIELEMENTENABLEDONLY != 0,
            st.tip_loaded,
        )
    };

    // ── 顶栏 ──────────────────────────────────────────────
    let bar = RECT { left: 0, top: 0, right: cw, bottom: 44 };
    let bbg = CreateSolidBrush(COLORREF(0x00EF_EF_EF));
    FillRect(hdc, &bar, bbg);
    let _ = DeleteObject(bbg.into());

    draw_text(
        hdc,
        &status,
        RECT { left: 16, top: 6, right: cw - 16, bottom: 24 },
        COLORREF(0x0030_3030),
        DT_LEFT | DT_TOP | DT_SINGLELINE | DT_NOPREFIX,
    );

    let hint = if !tip {
        note.clone()
    } else if uil {
        format!("UILess 模式（flags 已置位）—— 候选由本窗口绘制。{note}")
    } else if active {
        "对照组：flags=0，TIP 会自己弹候选框；本窗口收到 BeginUIElement 说明 TIP 仍在询问宿主。".to_string()
    } else {
        format!("等待 TIP 请求显示候选。{note}")
    };
    let hint_color = if !tip {
        COLORREF(0x0020_20C0) // 红
    } else {
        COLORREF(0x0060_6060)
    };
    draw_text(
        hdc,
        &hint,
        RECT { left: 16, top: 24, right: cw - 16, bottom: 42 },
        hint_color,
        DT_LEFT | DT_TOP | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
    );

    // ── 文档正文 ──────────────────────────────────────────
    if doc.is_empty() {
        draw_text(
            hdc,
            "（在这里打拼音，例如 limkyr）",
            RECT { left: TEXT_LEFT, top: TEXT_TOP, right: cw - 16, bottom: TEXT_TOP + LINE_H },
            COLORREF(0x00A0_A0A0),
            DT_LEFT | DT_TOP | DT_SINGLELINE | DT_NOPREFIX,
        );
    } else {
        draw_text(
            hdc,
            &doc,
            RECT { left: TEXT_LEFT, top: TEXT_TOP, right: cw - 16, bottom: TEXT_TOP + LINE_H },
            COLORREF(0x0010_1010),
            DT_LEFT | DT_TOP | DT_SINGLELINE | DT_NOPREFIX,
        );
    }

    // 光标（用选区起点）
    let (caret, focused) = {
        let st = state::lock();
        (st.sel.0, true)
    };
    if focused {
        let prefix: String = doc.chars().take(caret.max(0) as usize).collect();
        let x = TEXT_LEFT + text_width(hdc, &prefix);
        let line = CreateSolidBrush(COLORREF(0x0040_4040));
        FillRect(hdc, &RECT { left: x, top: TEXT_TOP + 3, right: x + 2, bottom: TEXT_TOP + LINE_H - 2 }, line);
        let _ = DeleteObject(line.into());
    }

    // ── 候选条（宿主自绘，这就是 UILess 的成果） ──────────
    if active && !cands.is_empty() {
        let start = pages.get(page).copied().unwrap_or(0) as usize;
        let end = pages
            .get(page + 1)
            .copied()
            .map(|v| v as usize)
            .unwrap_or(cands.len())
            .min(cands.len());

        let mut x = TEXT_LEFT;
        let y = TEXT_TOP + LINE_H + 10;
        for i in start..end {
            let label = format!("{}.{}", i - start + 1, cands[i]);
            let w = text_width(hdc, &label) + 22;
            let r = RECT { left: x, top: y, right: x + w, bottom: y + 34 };

            let (bg, fg) = if i == sel {
                (COLORREF(0x00D8_6F1D), COLORREF(0x00FF_FF_FF))
            } else {
                (COLORREF(0x00E6_E6_E6), COLORREF(0x0020_2020))
            };
            let br = CreateSolidBrush(bg);
            FillRect(hdc, &r, br);
            let _ = DeleteObject(br.into());

            draw_text(
                hdc,
                &label,
                RECT { left: r.left + 11, top: y + 8, right: r.right - 4, bottom: y + 30 },
                fg,
                DT_LEFT | DT_TOP | DT_SINGLELINE | DT_NOPREFIX,
            );
            x += w + 6;
        }
    }

    SelectObject(hdc, old_font);
    let _ = EndPaint(hwnd, &ps);
}
