fn main() {
    use feihe_core::{dict, paths, shuangpin::{self, Scheme}};
    paths::init("/tmp/feihe_test".into());
    for input in ["lqxbr","wfjmyul","wfjmy","lqxb","nih"] {
        let r = dict::lookup_input(input, false, 30);
        println!("--- 输入 {:9} 共{}键 n={}", input, input.len(), r.words.len());
        for w in r.words.iter().take(8) {
            let c = shuangpin::encode_phrase(Scheme::Xiaohe, w).unwrap_or_default();
            let flag = if c.len() > input.len()+1 { "  <== 超出+1" } else { "" };
            print!("{}({}){} ", w, c.len(), flag);
        }
        println!();
    }
}
