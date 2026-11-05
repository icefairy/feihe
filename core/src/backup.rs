//! 用户数据备份：**哪些文件算「用户的」**，以及导入之后怎么让它立刻生效。
//!
//! ## 为什么打包不在这里做
//!
//! core 的依赖政策是「只有 `tyme4rs`」，zip 得自己写一遍编码器（DEFLATE 要么引库
//! 要么实现，几百行且没有任何引擎价值）。打包/解包交给壳：Android 用
//! `java.util.zip`，以后 Windows 端接的时候用系统 API 或同样手写。core 只提供
//! **清单**（哪些文件、多大、版本号）和**重载**（导入后把内存态全部刷新）。
//!
//! 这样切分的好处是很实际的：备份的**内容**只有一份真值（`DATA_FILES`），
//! 而容器的格式错了只影响那一次导入导出，不会让引擎进入一个「半新半旧」的状态。
//!
//! ## `user.bin` 为什么不在清单里
//!
//! 它是 `user_dict.txt` 的编译产物，格式跟着词库版本走。跨版本导入一个旧 bin，
//! 最好的结果是「读不出来退回 txt」，最坏的结果是把旧格式当新格式解析、查出一堆
//! 乱码词。导入后重编译一次只要百来毫秒，没必要冒这个险。
//!
//! 同理 `feihe.log` 也不在里面：那是诊断日志，混进备份只会让人以为备份坏了。

use std::path::Path;

/// 要备份的数据文件（相对于数据目录）。
///
/// 顺序就是导出顺序，所以**只增不改名**：改名等于让旧备份永远导不回来。
/// 每一项都必须和对应模块里 `dir().join(..)` 的写法一字不差——这里是清单，
/// 那里是真值，两边不一致的后果是「备份成功了，但少了一个文件」。
pub const DATA_FILES: &[&str] = &[
    "config.ini",    // 设置（输入方式、方案、开关…）
    "user_dict.txt", // 个人词库（源文件；导入后由引擎重编译成 user.bin）
    "freq.txt",      // 词的动态词频（谁用得多久排前）
    "pinned.txt",    // 手动置顶的词
    "phrases.txt",   // 自造词（全拼存的，换方案不失效）
    "next.txt",      // 联想：用户自己打出来的词对
    "demoted.txt",   // 降权（Ctrl+点击压下去的词）
    "custom.txt",    // 自定义短语与变量
];

/// 压缩包里那份说明文件的文件名（由壳写进包根）。
pub const MANIFEST_NAME: &str = "MANIFEST.txt";

/// 这个名字是不是允许导入的数据文件。
///
/// **导入端必须用它做白名单**，而不是「解压到什么就写什么」：备份包是从外部来的，
/// 里面可能有 `../config.ini` 这种路径（zip slip），也可能有任意大文件。
/// 只认这里列出的名字，两个问题一起没了。
pub fn is_data_file(name: &str) -> bool {
    DATA_FILES.contains(&name)
}

/// 导出清单的正文：版本 + 时间 + 每个存在的文件及其字节数。
///
/// 时间用 `clock`（壳注入的时区），Android 上没有系统时区可用。
pub fn manifest() -> String {
    manifest_in(&crate::config::dir())
}

/// [`manifest`] 的实现，目录从参数进来——单测要用临时目录，
/// 不能碰真实数据目录（那里可能有用户数据，也可能是空的，两种都会让断言飘）。
fn manifest_in(dir: &Path) -> String {
    let t = crate::clock::now();
    let mut out = String::with_capacity(512);
    out.push_str("飞鹤输入法 · 用户数据备份\n");
    out.push_str(&format!("引擎版本 {}\n", crate::VERSION));
    out.push_str(&format!(
        "导出时间 {:04}-{:02}-{:02} {:02}:{:02}\n",
        t.year, t.month, t.day, t.hour, t.minute
    ));
    out.push_str("\n导入方式：设置 → 数据 → 导入备份（会覆盖当前数据）\n\n");
    out.push_str("文件\t字节\n");

    let mut total: u64 = 0;
    for name in DATA_FILES {
        if let Ok(m) = std::fs::metadata(dir.join(name)) {
            out.push_str(&format!("{name}\t{}\n", m.len()));
            total += m.len();
        }
    }
    out.push_str(&format!("合计\t{total}\n"));
    out
}

/// 导入之后把所有内存态重读一遍。
///
/// ## 顺序不是随便排的
///
/// 1. **配置先读**：它是热路径原子镜像的来源（`pinyin_mode` / `no_popup` /
///    `candidate_rows`），晚读一步，后面所有模块拿到的还是旧开关。
/// 2. **方案跟着配置走**：`config.scheme` 和 `dict::scheme()` 是两处状态，
///    只重读配置的话会出现「设置里是小鹤、查词还按微软」——界面上完全看不出来。
/// 3. **个人词库重编译**：备份里刻意没有 `user.bin`，所以必须由 txt 重新生成；
///    顺序在 `dict` 的其他动作之后没有区别，但要在返回前完成，否则壳刚提示
///    「导入成功」就已经能查出旧词。
///
/// 失败不返回错误：导入这个动作**已经写盘成功了**，重载只是让它生效。把「重载失败」
/// 报成「导入失败」会让用户以为数据没进去，于是再导一次——那才是真危险。
pub fn reload_all() {
    crate::config::load();
    crate::dict::set_scheme(crate::config::get().scheme);
    crate::freq::refresh();
    crate::pin::refresh();
    crate::phrase::refresh();
    crate::next::refresh();
    crate::demote::refresh();
    crate::custom::refresh();
    let _ = crate::dict::compile_user_bin();
    // 编译成功后 `user_bin` 已经重新映射；但如果包里没有 user_dict.txt，
    // 回退层（解析 txt 的堆索引）还是旧的，这里一并换掉。
    crate::dict::reload_overlay();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 清单本身不能有重复、不能带路径分隔符、扩展名要眼熟。
    ///
    /// 看着像废话，但这三样错了的表现都很远：重复项会让 zip 里出现同名的两个条目
    /// （解压出来是哪一个取决于实现），带 `/` 的项会让 Android 端写出数据目录。
    #[test]
    fn data_files_are_plain_unique_names() {
        let mut seen = std::collections::HashSet::new();
        for name in DATA_FILES {
            assert!(seen.insert(*name), "重复的备份文件：{name}");
            assert!(!name.contains('/') && !name.contains('\\'), "文件名字带路径：{name}");
            assert!(!name.starts_with('.'), "不该备份隐藏文件：{name}");
            assert!(
                name.ends_with(".txt") || name.ends_with(".ini"),
                "意外的扩展名：{name}"
            );
        }
    }

    /// 产物和日志都不许进清单（理由见模块文档）。
    #[test]
    fn build_output_is_not_backed_up() {
        assert!(!is_data_file("user.bin"), "user.bin 是产物，导入后应由 txt 重编译");
        assert!(!is_data_file("feihe.log"), "日志不该混进用户数据备份");
    }

    /// 白名单必须挡住路径穿越——备份包是**外部输入**，不能信里面的名字。
    #[test]
    fn whitelist_rejects_paths() {
        assert!(is_data_file("config.ini"));
        for bad in [
            "../config.ini",
            "/data/data/com.feihe.ime/files/config.ini",
            "config.ini.bak",
            "",
        ] {
            assert!(!is_data_file(bad), "不该接受 {bad:?}");
        }
    }

    /// 清单只列真实存在的文件，合计跟着算。
    #[test]
    fn manifest_lists_existing_files_and_total() {
        let dir = std::env::temp_dir().join(format!("feihe-manifest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录失败");

        // 只有一个文件：清单里只能出现它，合计等于它的大小
        std::fs::write(dir.join("pinned.txt"), "你好\n").expect("写文件失败");
        let m = manifest_in(&dir);
        assert!(m.contains("pinned.txt\t7"), "{m}");
        assert!(!m.contains("freq.txt"), "不存在的文件不该出现在清单里：{m}");
        assert!(m.contains("合计\t7"), "{m}");
        assert!(m.contains(crate::VERSION), "清单要带版本号：{m}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
