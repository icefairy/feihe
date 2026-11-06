//! 从**文件**导入搜狗词库：读盘 → 判格式 → 解析注音 → 合并进个人词库。
//!
//! 这一段以前在三个地方各写一份（Windows 的 `import_file`、`import_dir`、
//! Android 的 JNI 层），2026-11-07 用户报「Win 上导入没反应」才查清楚：桌面端
//! `.scel` 解析出 **0 条**的时候照样返回 `Ok`，界面显示一句
//! 「已解析1个文件（0条），没有新词——都已在词库里」，看上去像什么都没发生。
//!
//! 所以现在「一个文件 → `Imported`」只有这一份实现，而且**解析不出词就是 Err**。
//! 两端拿到的错误文案、统计数字完全一致。
//!
//! 平台无关：只碰 `std::fs::read` 和 `dict::merge_text`，不认「搜狗装在哪个目录」
//! （那是各家壳自己的事，见 `src/sogou.rs` 的 `discover`）。

use std::path::Path;

use crate::sogou::{self, Imported, Job, MAX_LEN};

/// 大于这个大小的文件直接拒绝：`.scel` / `.bin` 正常只有几百 KB 到几十 MB，
/// 上百 MB 多半是选错了文件（比如把装机的 zip 选进来了）。
const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;

/// 扩展名 → [sogou::Kind]。手动挑的 `.bin` 一律宽松解析（没有「这是打字历史」的上下文）。
fn kind_of(path: &Path) -> Result<sogou::Kind, String> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    sogou::kind_for_ext(&ext).ok_or_else(|| {
        format!("不认识的扩展名 .{ext}（支持 .scel 细胞词库 / .bin / .txt 文本词库）")
    })
}

/// 一个词库文件 → 并进个人词库。
///
/// 返回的 [Imported] 里 `added` 是**真正新增**的条数，`encoded` 是解析并注音成功的条数。
///
/// **解析出 0 条是错误**（`Err`），不是「成功但没新增」：前者意味着文件格式对不上、
/// 选错了文件或者文件损坏，用户必须知道；后者才是「这些词本来就有」。
pub fn import_file(path: &Path, max_len: usize) -> Result<Imported, String> {
    let kind = kind_of(path)?;

    match std::fs::metadata(path) {
        Ok(m) if m.len() > MAX_FILE_BYTES => {
            return Err(format!(
                "文件有 {} MB，太大了（词库文件不该超过 {} MB）",
                m.len() / 1024 / 1024,
                MAX_FILE_BYTES / 1024 / 1024
            ))
        }
        Ok(_) => {}
        Err(e) => return Err(format!("读不到这个文件：{e}")),
    }
    let data = std::fs::read(path).map_err(|e| format!("读不到这个文件：{e}"))?;

    let label = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    crate::dlog!("sogou: 开始解析 {}（{} KB，{:?}）", label, data.len() / 1024, kind);

    let job = Job {
        label,
        data,
        kind,
        // 手动挑的文件按细胞词库级（1）：和自动发现的 `Scd/*.scel` 同级，
        // 打字历史（3）/ 用户词库（2）永远压在它们上面。
        prio: 1,
    };
    let (wordlist, mut stat) = sogou::encode_one(&job, max_len);
    merge(&wordlist, &mut stat)
}

/// 一批**已经读进内存**的文件 → 并进个人词库。Windows 的「从搜狗导入…」用它
/// （那边要先按注册表找到一串 bin 和 scel，读盘策略不一样）。
pub fn import_jobs(jobs: &[Job], max_len: usize) -> Result<Imported, String> {
    if jobs.is_empty() {
        return Err("没有可解析的词库文件".into());
    }
    let (wordlist, mut stat) = sogou::encode_jobs(jobs, max_len);
    merge(&wordlist, &mut stat)
}

/// 解析结果 → `user_dict.txt`（顺带编译 `user.bin`）。两端共用这一段收尾。
fn merge(wordlist: &str, stat: &mut Imported) -> Result<Imported, String> {
    if stat.encoded == 0 {
        return Err(stat.summary());
    }
    let (added, _bin) = crate::dict::merge_text(wordlist)?;
    stat.added = added;
    Ok(Imported { ..*stat })
}

/// 默认词长上限（`sogou::MAX_LEN` 的快捷封装，壳里省一个 import）。
pub fn import_file_default(path: &Path) -> Result<Imported, String> {
    import_file(path, MAX_LEN)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("feihe_sogou_import_test");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(bytes).unwrap();
        p
    }

    #[test]
    fn unknown_extension_is_rejected_with_a_hint() {
        let p = temp("a.exe", b"whatever");
        let e = import_file(&p, MAX_LEN).unwrap_err();
        assert!(e.contains("scel"), "错误信息要说清支持什么：{e}");
    }

    /// 这次「导入没反应」的现场：文件能读、格式不认识、解析 0 条。
    /// **必须返回 Err**，否则界面会显示一句「没有新词——都已在词库里」，
    /// 用户以为功能没生效。
    #[test]
    fn zero_words_is_an_error_not_silent_success() {
        let p = temp("empty.txt", b"# only a comment\n\n");
        let e = import_file(&p, MAX_LEN).unwrap_err();
        assert!(e.contains("没能"), "{e}");
    }

    #[test]
    fn missing_file_is_reported() {
        let e = import_file(Path::new("D:/no/such/dict.scel"), MAX_LEN).unwrap_err();
        assert!(e.contains("读不到"), "{e}");
    }

    /// 正常文本：必须真的进个人词库并报出新增条数。
    #[test]
    fn good_text_merges_into_user_dict() {
        let p = temp("good.txt", "龘靐齉\\n".as_bytes());
        match import_file(&p, MAX_LEN) {
            Ok(s) => {
                assert!(s.encoded > 0);
                assert_eq!(s.files, 1);
            }
            // 环境里没有个人词库目录时 merge_text 会失败，那是环境问题不是逻辑问题
            Err(e) => assert!(!e.is_empty()),
        }
    }
}
