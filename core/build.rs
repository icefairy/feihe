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
// 联想表的编码器就住在运行时读取器里（nextbin.rs）：编码 / 解码共用同一份格式定义，
// 不会出现「这边加了个字段、那边没跟上」。
#[allow(dead_code)]
#[path = "src/nextbin.rs"]
mod nextbin;
#[allow(dead_code)]
#[path = "src/dictcats.rs"]
mod dictcats;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    gen_dict_bin(&manifest, &out);
    gen_next_bin(&manifest, &out);
}

// ─────────────────────────────────────────────────────────────
// 联想表二进制生成
// ─────────────────────────────────────────────────────────────

/// 把 src/next_builtin.txt 编成 next.bin（零解析，见 src/nextbin.rs）。
///
/// 为什么不在运行时解析：309 KB 文本解析成 HashMap 要 5.8 ms + 2.1 MB 堆，而这段
/// 代码跑在「第一次上屏」的 UI 线程上。构建期多做几毫秒，换运行时零成本。
///
/// 校验只做「这一行能不能用」，不做「词好不好」——词的质量是 gen_next.py 的事。
/// 但会检查同一头词的权重是否递减：文件顺序就是运行时的顺序（二进制里不存权重），
/// 乱序会在无声无息中改变联想排序。
fn gen_next_bin(manifest: &std::path::Path, out: &std::path::Path) {
    println!("cargo:rerun-if-changed=src/next_builtin.txt");
    let path = manifest.join("src/next_builtin.txt");
    let text = std::fs::read_to_string(&path).unwrap_or_default();

    let mut rows: Vec<(String, Vec<String>)> = Vec::new();
    let mut warned = false;
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((head, rest)) = line.split_once('\t') else {
            continue;
        };
        if !is_word(head) {
            continue;
        }
        let mut words: Vec<String> = Vec::new();
        let mut prev_weight = u32::MAX;
        let mut it = rest.split_whitespace();
        while let (Some(w), Some(n)) = (it.next(), it.next()) {
            let Ok(weight) = n.parse::<u32>() else {
                continue;
            };
            if weight == 0 || !is_word(w) {
                continue;
            }
            if weight > prev_weight && !warned {
                warned = true;
                println!(
                    "cargo:warning=next_builtin.txt 里「{head}」的后续权重不是递减的；\
                     运行时按文件顺序取，乱序会改变联想排序"
                );
            }
            prev_weight = weight;
            words.push(w.to_string());
        }
        if !words.is_empty() {
            rows.push((head.to_string(), words));
        }
    }

    let bytes = nextbin::encode(&rows);
    std::fs::write(out.join("next.bin"), &bytes).expect("写 next.bin 失败");
    let pairs: usize = rows.iter().map(|r| r.1.len()).sum();
    println!(
        "cargo:warning=next.bin: {} 头词 / {} 对, {} KB",
        rows.len(),
        pairs,
        bytes.len() / 1024
    );
}

/// 联想表里合法的词：1..=8 个汉字。和运行时 next::is_word 同一条规则。
fn is_word(s: &str) -> bool {
    let mut n = 0;
    for c in s.chars() {
        let o = c as u32;
        let han = (0x4E00..=0x9FFF).contains(&o)
            || (0x3400..=0x4DBF).contains(&o)
            || (0xF900..=0xFAFF).contains(&o);
        if !han {
            return false;
        }
        n += 1;
        if n > 8 {
            return false;
        }
    }
    n > 0
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

    // **玄学分类单独成包**，不进 `base.bin`。
    //
    // 这些词的读音对外人没有意义（`庚` `戊` `值符` `旺相休囚死`），混在通用词库里
    // 唯一的效果是把单键 / 短码的重码变多。抽成独立的 `xuanxue.bin` 之后，
    // 运行期由 `config::xuanxue()` 一个开关决定扫不扫——不开就一个字节的查询成本都
    // 没有（`dictbin::xuanxue()` 在开关关着时**压根不碰**那份数据）。
    //
    // 为什么用一个包而不是十来个分类包：这摊词一共 700 来条、彼此高度相关
    // （干支 + 易经 + 奇门 + 六壬 + 六爻 + 神煞是同一批用户），拆细只会让设置窗里
    // 多出几个没人看得懂的勾选框。词库里的其余分类（中医、IT、地名…）留在通用
    // 词库里——它们是「偶尔能打到就好」，不是「要么全要要么全不要」。
    // 清单在 `src/dictcats.rs`，和 `dict.rs` 的测试共用同一份：漏登记的后果是
    // 「词生成出来了但开关打开也打不出来」，而且没有任何报错。
    let (xuan_sources, general_sources): (Vec<String>, Vec<String>) = sources
        .into_iter()
        .partition(|f| dictcats::is_xuanxue(f.rsplit('/').next().unwrap_or(f)));

    // 通用词库（base.bin）
    let mut entries: Vec<(String, String)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for f in &general_sources {
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

    // 玄学词库（xuanxue.bin）
    //
    // 去重集合**单独一份**：玄学词和通用词库里的词允许重叠（`节` `气` 这类既是
    // 天干地支相关、又是常用字），运行期查询时同名候选由 `collect` 的 `seen` 兜着，
    // 不会重复出现。共用集合反而会让先生成的通用词把玄学词「吃掉」，开关打开也
    // 查不到东西。
    //
    // 排序也要**按通用词库的顺序**：`rank_key` 的第二部分就是词库原始行号（≈ 词频），
    // 所以把玄学词按文件顺序排好，等于让它们落在通用词之后、同码时排在后面。
    let mut x_entries: Vec<(String, String)> = Vec::new();
    let mut x_seen = std::collections::HashSet::new();
    for f in &xuan_sources {
        println!("cargo:rerun-if-changed={f}");
        let Ok(text) = std::fs::read_to_string(manifest.join(f)) else {
            continue;
        };
        dictenc::push_entries(&text, &mut x_seen, &mut x_entries);
    }
    let xbuilt = dictenc::build(&x_entries);
    std::fs::write(out.join("xuanxue.bin"), &xbuilt.bytes).expect("写 xuanxue.bin 失败");
    println!(
        "cargo:warning=xuanxue.bin: {} 条, {} KB（默认不启用，见 config::xuanxue）",
        xbuilt.count,
        xbuilt.bytes.len() / 1024
    );
}
