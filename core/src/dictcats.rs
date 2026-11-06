//! 分类词库的清单：`src/extra_*.txt` 里哪些属于「玄学」这一摊。
//!
//! ## 为什么单独一个文件
//!
//! 这份清单被两边用：
//!
//! - `core/build.rs`：把玄学类编成独立的 `xuanxue.bin`，不进 `base.bin`
//! - `core/src/dict.rs` 的测试：玄学词不在默认查询路径上，得单独验「拼音编对了」
//!
//! 写两份的后果是「加了分类但忘了在另一处登记」——表现是词生成出来了、开关打开
//! 也打不出来，而且没有任何报错。这和 `dictenc.rs` 被 `#[path]` 共用一个道理：
//! 同一份事实只能有一处定义。
//!
//! ## 为什么只有「玄学」一个分类
//!
//! 见 `config::Config::xuanxue` 的说明：这摊词一共 600 多条、彼此高度相关，
//! 而其余分类（中医、IT、地名…）属于「偶尔能打到就好」，留在通用词库里正好。

/// 玄学分类的源文件名（不带路径，都在 `src/` 下）。
pub const XUANXUE: [&str; 6] = [
    "extra_ganzhi.txt",
    "extra_yijing.txt",
    "extra_shensha.txt",
    "extra_qimen.txt",
    "extra_liuren.txt",
    "extra_liuyao.txt",
];

/// 这个源文件是不是玄学分类。
pub fn is_xuanxue(file_name: &str) -> bool {
    XUANXUE.contains(&file_name)
}
