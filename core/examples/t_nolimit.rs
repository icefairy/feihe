fn main() {
    use feihe_core::{dict, paths};
    paths::init("/tmp/feihe_test".into());
    // 最坏情况：单键 / 双键，候选最多的码
    let big = usize::MAX;
    for code in ["a","l","i","u","v","y","lq","shi","yi"] {
        let t = std::time::Instant::now();
        let n = dict::lookup_input(code, false, big).words.len();
        let d = t.elapsed();
        println!("{:4} limit=MAX -> {:5} 条  耗时 {:>8.3}ms", code, n, d.as_secs_f64()*1000.0);
    }
    println!("--- 对比 limit=200 ---");
    for code in ["a","l","lq"] {
        let t = std::time::Instant::now();
        let n = dict::lookup_input(code, false, 200).words.len();
        let d = t.elapsed();
        println!("{:4} limit=200 -> {:5} 条  耗时 {:>8.3}ms", code, n, d.as_secs_f64()*1000.0);
    }
}
