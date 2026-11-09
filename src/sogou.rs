//! 从搜狗拼音导入用户词库（`tools/import_sogou.py` 的运行期版本）。
//!
//! 原来这是开发者在自己机器上跑一次脚本、把结果写进 `src/user_dict.txt` 再重新
//! 编译 DLL。用户换不了自己的词库，等于「从搜狗迁移」这条路只有作者能走。现在
//! 同样的解析逻辑搬进 DLL，设置窗里点一下就把搜狗的打字历史并进个人词库。
//!
//! ## 数据源
//!
//! ```text
//! <搜狗安装目录>/用户数据/SogouPY.users/<id>/sgim_gd_usrbg.bin   打字历史
//! <搜狗安装目录>/用户数据/SogouPY.users/<id>/sgim_gd_usr.bin     用户词库
//! <搜狗安装目录>/Scd/*.scel                                      细胞词库
//! <用户数据目录>/SogouPY/users/<id>/UserDictionary.txt            手动导出
//! ```
//!
//! ## 注音
//!
//! 搜狗的 bin/scel **只有词、没有拼音**，得自己注。策略分两级：
//!
//! 1. 先查编译进 DLL 的基础词库（`dictbin::get()`）——那里是「词 → 全拼」，
//!    多音字天然正确（`重庆` 就该是 `chong qing`）；
//! 2. 查不到才逐字注音，单字取基础词库里的**第一条**读音。基础词库是按词频排的，
//!    同一个字的多条读音里第一条就是最常用的那个。
//!
//! 逐字注音会错（`银行` 可能注成 `yin xing`），但它只作用于基础词库没有的词，
//! 而且错了也只是那个词排得靠后、或者敲错码——不会污染别的词。
//!
//! ## 为什么不用 Python 脚本那套「校验和」逻辑的分歧版本
//!
//! `sgim_gd_usrbg.bin` 每条记录末尾有个 `字节长 + 6` 的字段，可以当校验和用，
//! 能把字节错位产生的假词全部滤掉；`sgim_gd_usr.bin` 没有这个字段，只能宽松
//! 匹配、靠「必须能注音」兜底。两者的解析函数因此分开，`bin_strict` / `bin_loose`。

use std::path::{Path, PathBuf};

/// 收录词的最大字数。定义在 core（`feihe_core::sogou::MAX_LEN`），这里转发。
pub use feihe_core::sogou::MAX_LEN;

// ─────────────────────────────────────────────────────────────
// 结果
// ─────────────────────────────────────────────────────────────

/// 一次导入的统计。搬进 core 了（`feihe_core::sogou::Imported`），这里只做转发——
/// 两端要拿到一样的数字，结构体不能有两个。
pub use feihe_core::sogou::Imported;


/// 每种文件怎么解析、优先级多少。**定义在 core 里**（`feihe_core::sogou::Kind`），
/// 这里只是转发：解析器在 core，壳和 core 必须认同一套枚举。
pub use feihe_core::sogou::Kind;

struct Job {
    label: String,
    path: PathBuf,
    kind: Kind,
    prio: u8,
}

/// 读注册表默认值（搜狗把自己的安装目录存在 `SogouInput` 键的默认值里）。
///
/// 三个位置都试：64 位视图看不到 32 位安装的键，反过来也一样，而搜狗是 32 位
/// 程序、绝大多数人装在 WOW6432Node 下。
fn reg_install_dir(hive: isize, subkey: &str) -> Option<PathBuf> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegGetValueW, HKEY, RRF_RT_REG_SZ,
    };

    let key: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * std::mem::size_of::<u16>()) as u32;
    let rc = unsafe {
        RegGetValueW(
            HKEY(hive as *mut core::ffi::c_void),
            PCWSTR(key.as_ptr()),
            PCWSTR::null(),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut core::ffi::c_void),
            Some(&mut size),
        )
    };
    if rc != ERROR_SUCCESS {
        return None;
    }
    let n = ((size as usize) / 2).min(buf.len());
    let s = String::from_utf16_lossy(&buf[..n]);
    let s = s.trim_end_matches('\u{0}');
    let p = PathBuf::from(s);
    p.is_dir().then_some(p)
}

/// 可能存放搜狗数据的根目录（去重、只留真实存在的）。
fn roots() -> Vec<(String, PathBuf)> {
    // HKEY_LOCAL_MACHINE = 0x80000002，HKEY_CURRENT_USER = 0x80000001
    const HKLM: isize = 0x8000_0002u32 as i32 as isize;
    const HKCU: isize = 0x8000_0001u32 as i32 as isize;

    let mut cands: Vec<(String, PathBuf)> = Vec::new();
    for (hive, tag, sub) in [
        (HKLM, "HKLM", r"SOFTWARE\WOW6432Node\SogouInput"),
        (HKLM, "HKLM", r"SOFTWARE\SogouInput"),
        (HKCU, "HKCU", r"SOFTWARE\SogouInput"),
    ] {
        if let Some(p) = reg_install_dir(hive, sub) {
            cands.push((tag.to_string(), p));
        }
    }
    for env in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Ok(base) = std::env::var(env) {
            cands.push((env.to_string(), Path::new(&base).join("SogouInput")));
        }
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        cands.push(("APPDATA".into(), Path::new(&appdata).join("SogouPY")));
        cands.push(("APPDATA".into(), Path::new(&appdata).join("SogouPY.users")));
        cands.push(("APPDATA".into(), Path::new(&appdata).join("SogouInput")));
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        cands.push(("LOCALAPPDATA".into(), Path::new(&local).join("SogouPY")));
        cands.push(("LOCALAPPDATA".into(), Path::new(&local).join("SogouPY.users")));
    }

    let mut seen = std::collections::HashSet::new();
    cands
        .into_iter()
        .filter(|(_, p)| p.is_dir())
        .filter(|(_, p)| seen.insert(p.clone()))
        .collect()
}

fn dir_has_sgim(dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else { return false };
    rd.flatten()
        .any(|e| e.file_name().to_string_lossy().starts_with("sgim_"))
}

/// 一个「用户数据目录」里的文件（`sgim_gd_usrbg.bin` / `sgim_gd_usr.bin` / 导出的 txt）
fn jobs_in_user_dir(dir: &Path, label: &str, out: &mut Vec<Job>) {
    let bg = dir.join("sgim_gd_usrbg.bin");
    if bg.is_file() {
        out.push(Job {
            label: format!("{label}/usrbg"),
            path: bg,
            kind: Kind::BinStrict,
            prio: 3,
        });
    }
    let usr = dir.join("sgim_gd_usr.bin");
    if usr.is_file() {
        out.push(Job {
            label: label.to_string(),
            path: usr,
            kind: Kind::BinLoose,
            prio: 2,
        });
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let low = name.to_lowercase();
        if low.ends_with(".txt") && (low.contains("dict") || name.contains("词库")) {
            out.push(Job {
                label: format!("{label}/{name}"),
                path: e.path(),
                kind: Kind::Text,
                prio: 1,
            });
        }
    }
}

fn jobs_in_scd_dir(dir: &Path, label: &str, out: &mut Vec<Job>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.to_lowercase().ends_with(".scel") {
            out.push(Job {
                label: format!("{label}/{name}"),
                path: e.path(),
                kind: Kind::Scel,
                prio: 1,
            });
        }
    }
}

/// 自动发现：从注册表 / 环境变量推搜狗的安装目录和用户数据目录。
fn discover() -> Result<(Vec<(String, PathBuf)>, Vec<Job>), String> {
    let roots = roots();
    if roots.is_empty() {
        return Err(format!(
            "没找到搜狗输入法。\n\n如果它装在别的盘，可以用「选择文件夹…」手动指定，\
             一般是：\n<搜狗安装目录>\\用户数据\\SogouPY.users\\<一串数字>"
        ));
    }
    let mut jobs = Vec::new();
    for (label, root) in &roots {
        // 版本之间层级不一样：有的直接是用户目录，有的下面还套 SogouPY.users/<id>。
        // 先试「root 自己就是」，再试三层相对路径——猜错层级只是多几次 stat。
        if dir_has_sgim(root) {
            jobs_in_user_dir(root, label, &mut jobs);
        }
        // <root>/用户数据/SogouPY.users/<id>/
        for rel in ["用户数据\\SogouPY.users", "users", "SogouPY.users"] {
            let base = root.join(rel);
            let Ok(rd) = std::fs::read_dir(&base) else { continue };
            for e in rd.flatten() {
                let sub = e.path();
                if sub.is_dir() && dir_has_sgim(&sub) {
                    let tag = format!("{label}:{rel}/{}", e.file_name().to_string_lossy());
                    jobs_in_user_dir(&sub, &tag, &mut jobs);
                }
            }
        }
        let scd = root.join("Scd");
        if scd.is_dir() {
            jobs_in_scd_dir(&scd, &format!("{label}:Scd"), &mut jobs);
        }
    }
    Ok((roots, jobs))
}

/// 手动指定的目录：既可能是「用户数据目录」本身，也可能是它的上一层。
///
/// 不假设层级——把目录自己、它的子目录、以及 `Scd` 都扫一遍，用户选错一层也
/// 能导入，比弹一个「这里没有搜狗数据」有用得多。
fn jobs_in_dir(dir: &Path) -> Vec<Job> {
    let mut jobs = Vec::new();
    if dir_has_sgim(dir) {
        jobs_in_user_dir(dir, &dir.display().to_string(), &mut jobs);
    }
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let sub = e.path();
            if sub.is_dir() && dir_has_sgim(&sub) {
                jobs_in_user_dir(&sub, &sub.display().to_string(), &mut jobs);
            }
        }
    }
    jobs_in_scd_dir(dir, &dir.display().to_string(), &mut jobs);
    let scd = dir.join("Scd");
    if scd.is_dir() {
        jobs_in_scd_dir(&scd, &format!("{}/Scd", dir.display()), &mut jobs);
    }
    jobs
}

// ─────────────────────────────────────────────────────────────
// 主流程
// ─────────────────────────────────────────────────────────────

fn run(jobs: &[Job], max_len: usize) -> Result<Imported, String> {
    if jobs.is_empty() {
        return Err("没有发现可解析的搜狗词库文件".into());
    }

    // 这一层只负责读盘，解析 / 注音 / 合并全在 `feihe_core::sogou_import`——
    // 和 Android 的「从搜狗导入」走同一段代码（2026-11-07 之前各写一份，
    // 桌面端那份解析出 0 条还当成功，用户看到的就是「点了没反应」）。
    let mut loaded = Vec::with_capacity(jobs.len());
    for job in jobs {
        let Ok(data) = std::fs::read(&job.path) else { continue };
        // 一个文件一行：用户报「导入后词没变多」时，这份日志能直接看出是
        // 哪个文件没解析出词（搜狗版本一升，bin 的布局就可能变）。
        crate::diag::log(format_args!("sogou: 读入 {}（{} KB）", job.label, data.len() / 1024));
        // 注意这里是 `feihe_core::sogou::Job`（带 data），不是本地那个带 path 的
        loaded.push(feihe_core::sogou::Job {
            label: job.label.clone(),
            data,
            kind: job.kind,
            prio: job.prio,
        });
    }
    let r = feihe_core::sogou_import::import_jobs(&loaded[..], max_len);
    match &r {
        Ok(s) => crate::diag::log(format_args!(
            "sogou: {} 个文件 / 解析 {} 条 → 可注音 {} 条（词库 {}、逐字 {}），跳过 {}，新增 {}",
            s.files, s.scanned, s.encoded, s.from_base, s.from_char, s.skipped, s.added
        )),
        Err(e) => crate::diag::log(format_args!("sogou: 失败 —— {e}")),
    }
    // `.scel` 里没有拼音，全靠 `Encoder` 查基础词库 + 逐字注音。查不到的字会被丢，
    // 全丢就是「一条都注不出音」——单独说清楚，否则用户只会收到一句笼统的
    // 「没能从这个词库文件里解析出词」，方向完全错了（文件是对的，注音挂了）。
    if let Err(e) = &r {
        if e.contains("解析出词") {
            return Err(format!(
                "{e}\n（也可能是词库里的字基础词库都没有读音，先看 feihe.log 里 sogou: 开头几行。）"
            ));
        }
    }
    r
}

/// 自动发现并导入。
pub fn import_auto(max_len: usize) -> Result<Imported, String> {
    let (_roots, jobs) = discover()?;
    if jobs.is_empty() {
        return Err(
            "找到了搜狗目录，但里面没有用户词库文件。\n\n可能的原因：一次都没用搜狗打过字，\
             或者它还没把词库落盘。"
                .into(),
        );
    }
    run(&jobs, max_len)
}

/// 导入用户手动挑选的**单个文件**——典型场景：从搜狗词库网站下载了一个
/// 细胞词库 .scel，跟本机装没装搜狗完全无关。
pub fn import_file(path: &Path, max_len: usize) -> Result<Imported, String> {
    // 也不自己读盘：`sogou_import::import_file` 会把「读不到 / 太大 / 扩展名不认识 /
    // 一条词都没解析出来」全部变成明确的错误。
    let r = feihe_core::sogou_import::import_file(path, max_len);
    match &r {
        Ok(s) => crate::diag::log(format_args!(
            "sogou: {} → 解析 {} 条，可注音 {} 条，新增 {}",
            path.display(),
            s.scanned,
            s.encoded,
            s.added
        )),
        Err(e) => crate::diag::log(format_args!("sogou: {} 失败 —— {e}", path.display())),
    }
    r
}

/// 从用户手动指定的目录导入。
pub fn import_dir(dir: &Path, max_len: usize) -> Result<Imported, String> {
    let jobs = jobs_in_dir(dir);
    if jobs.is_empty() {
        return Err(format!(
            "{} 里没有搜狗词库文件（找 sgim_*.bin / *.scel / 导出的 txt）。",
            dir.display()
        ));
    }
    run(&jobs, max_len)
}

// ─────────────────────────────────────────────────────────────
// 测试
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 这些解析 / 注音的细节测试在**搬走之前**住在壳里，现在归 core 了
    /// （`feihe_core::sogou::tests`）。留在这里的只有「壳自己的那部分」：
    /// 目录发现、文件选路、以及「一个文件进来能不能真的落进个人词库」。
    ///
    /// 别把解析用例再抄回来——抄回来就是两份要在两个 crate 里同步的测试。

    #[test]
    fn jobs_in_dir_finds_scel_even_one_level_up() {
        // 目录发现的取舍：用户选错一层也能导入，比弹「这里没有搜狗数据」有用。
        let dir = std::env::temp_dir().join("feihe_win_sogou_dir_test");
        let scd = dir.join("Scd");
        std::fs::create_dir_all(&scd).unwrap();
        std::fs::write(scd.join("经济学.scel"), b"SougouPY").unwrap();

        let jobs = jobs_in_dir(&dir);
        assert!(
            jobs.iter().any(|j| matches!(j.kind, Kind::Scel)),
            "Scd 下的 .scel 要被发现：{:?}",
            jobs.iter().map(|j| j.label.clone()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn import_file_rejects_unknown_extension() {
        // 导入要写个人词库 → 必须有数据目录。解析器是 `test_guard()` 注册的，
        // 不拿这把锁就会在 core 侧 panic（`数据目录未初始化`）——而且只在**并行跑
        // 全量测试**时偶发：别的用例先跑过就正常，自己跑就一定红。
        let _g = crate::config::test_guard();
        // 「导入没反应」的第一个可能：文件选错了。错误必须是明确的一句中文。
        let p = std::env::temp_dir().join("feihe_win_sogou_bad.exe");
        std::fs::write(&p, b"nope").unwrap();
        let e = import_file(&p, MAX_LEN).unwrap_err();
        assert!(e.contains("scel"), "{e}");
    }

    #[test]
    fn import_file_does_not_silently_succeed_on_zero_words() {
        let _g = crate::config::test_guard();
        // 用户报「Win 上导入没反应」的核心现场：文件能读、但一条词都解析不出来。
        // 这时必须是 Err——否则设置窗会显示「没有新词——都已在词库里」，
        // 看起来跟成功一模一样。
        let p = std::env::temp_dir().join("feihe_win_sogou_empty.scel");
        std::fs::write(&p, b"not a scel at all").unwrap();
        let e = import_file(&p, MAX_LEN).unwrap_err();
        assert!(e.contains("没能"), "要说清楚是解析不出来：{e}");
    }
}
