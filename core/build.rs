// 词库二进制生成。放在 `feihe-core` 里，因为**词库属于 core**：
// `dictbin::RAW = include_bytes!(concat!(env!("OUT_DIR"), "/base.bin"))` 要在
// core 的 OUT_DIR 里找到它。壳那边的 `build.rs` 只剩 windres 打图标。
//
// 词库的**源文件**仍在仓库根的 `src/`（`tools/gen_dict.py` / `gen_extra.py` 写那里），
// 所以下面的路径都是 `../src/...`。将来要把数据也搬进 core，只需改这一个前缀。
use std::path::PathBuf;

// 直接复用运行时的编码器，避免双拼逻辑写两份、改一处忘一处。
// `src/shuangpin.rs` / `src/dictenc.rs` 只依赖 std，可以原样拉进 build script 编译。
// **改它之前先确认没有同名文件**，否则会静默编到旧的那份去。
#[allow(dead_code)]
#[path = "src/shuangpin.rs"]
mod shuangpin;
#[allow(dead_code)]
#[path = "src/dictenc.rs"]
mod dictenc;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    gen_dict_bin(&manifest, &out);
}

// ─────────────────────────────────────────────────────────────
// 词库二进制生成
// ─────────────────────────────────────────────────────────────

/// 把 `src/user_dict.txt` + `src/dict.txt` 编译成扁平二进制。
///
/// 为什么不用 `include_str!` + 运行时编码：
/// - 运行时要把 8 万条全拼音节编码成双拼码并排序，`Activate` 里要付 81ms，**每个进程各付一次**
/// - 编码后是堆上的一堆 `Arc`/`Box`，约 9.2MB **私有**内存，同样是每个进程各一份
///
/// 预编码之后，三套码都在构建期算好，运行时只是「读两个 u32 数组 + 二分」，
/// 而 `include_bytes!` 的数据落在 DLL 的 `.rdata` 段里——这一段由 PE 加载器 mmap，
/// **多个进程共享同一份物理页**，实际触达的页数只跟查询次数有关。
fn gen_dict_bin(manifest: &std::path::Path, out: &std::path::Path) {
    // 优先级：用户词在前（同码时排在前面，实现「用户词优先」），
    // 然后是基础词库，最后是人工补充的常用词（只含前两份里缺的词）。
    //
    // `FEIHE_NO_USER_DICT=1` 时跳过 user_dict.txt。
    // 打包分发必须用它：那份文件是从本机搜狗导出的个人打字记录，
    // 一旦并进 base.bin 就随 DLL 一起发出去了，而且是从二进制里抠不出来的。
    let skip_user = std::env::var("FEIHE_NO_USER_DICT").ok().as_deref() == Some("1");
    println!("cargo:rerun-if-env-changed=FEIHE_NO_USER_DICT");

    let mut sources: Vec<String> = Vec::new();
    if skip_user {
        println!("cargo:warning=FEIHE_NO_USER_DICT=1：已排除 user_dict.txt（发布构建）");
    } else {
        sources.push("../src/user_dict.txt".to_string());
    }
    sources.push("../src/dict.txt".to_string());
    sources.push("../src/extra_dict.txt".to_string());
    // 按分类自动生成的补充词表（`tools/gen_extra.py` 产出）。
    //
    // 分文件而不是全塞进 `extra_dict.txt`：那份是手工挑的，混进上万行生成内容
    // 之后就没人愿意维护了。顺序在这里无关紧要——去重是先到先得，这些文件都在
    // `dict.txt` 之后，所以不会抢走基础词库里的词。
    //
    // 这里**扫目录**而不是写死清单：分类还在增加，写死就得在两处同步（`gen_extra.py`
    // 的 `CATS` 和这里），漏了的表现是「词生成出来了但打不出来」，很难查。
    // `extra_dict.txt` 是手工表，上面已经 push 过，遍历时排除。
    let mut generated: Vec<String> = std::fs::read_dir(manifest.join("../src"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with("extra_") && n != "extra_dict.txt" && n.ends_with(".txt"))
        .collect();
    generated.sort();
    sources.extend(generated.into_iter().map(|n| format!("../src/{n}")));

    let mut entries: Vec<(String, String)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for f in sources {
        println!("cargo:rerun-if-changed={f}");
        let Ok(text) = std::fs::read_to_string(manifest.join(f)) else {
            continue;
        };
        dictenc::push_entries(&text, &mut seen, &mut entries);
    }
    if entries.is_empty() {
        println!("cargo:warning=词库为空，base.bin 只会有头部");
    }

    // 编码逻辑与运行时生成 user.bin 共用 `src/dictenc.rs`，不在这里重写一遍。
    let built = dictenc::build(&entries);

    let path = out.join("base.bin");
    std::fs::write(&path, &built.bytes).expect("写 base.bin 失败");
    println!(
        "cargo:warning=base.bin: {} 条, {} KB (小鹤 {} / 微软 {} 可编码)",
        built.count,
        built.bytes.len() / 1024,
        built.xh,
        built.ms
    );
}
