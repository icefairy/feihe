//! 阿拉伯数字 → 中文写法：读法（一百二十三）、财务大写（壹佰贰拾叁）、
//! 小数读法（一百二十三点五）、金额（一百二十三元五角）。
//!
//! 全部是纯函数、不碰任何全局状态，所以能直接单测。
//!
//! ## 读法规则（容易写错的几条）
//!
//! - **四个零只读一个**：`10005` → 一万零五，不是一万零零零零五。
//! - **末尾的零不读**：`1200` → 一千二百。
//! - **节与节之间要补零**：`100200` → 十万零二百（万节和个节之间有空洞）；
//!   但 `120000` → 十二万（个节整个是零，什么都不补）。
//! - **「一十」只有开头才省**：`15` → 十五、`110` → 一百一十（中间的不省）。
//!   财务大写一律不省：`15` → 壹拾伍。
//! - **「亿」不是四位数一节**：`1234567890123` 读作「一万二千三百四十五亿六千七百八十九万
//!   零一百二十三」——高五位整体读出再接「亿」，不能按「万亿」当单位。
//!
//! ## 金额
//!
//! 元、角、分三段。角分都为零写「整」；有分无角在元后补「零」（壹佰贰拾叁元零伍分）；
//! 不足一元不写「元」（五角、伍分），这是票据写法。

/// 一套中文数字的用字。
pub struct Style {
    /// 0–9
    digits: [&'static str; 10],
    /// 个、十、百、千
    units: [&'static str; 4],
    /// 整串以「一十」开头时省掉「一」。财务大写不省。
    drop_one_before_ten: bool,
}

/// 年份逐位读用的 0–9。
///
/// 零写作「〇」（二〇二六年），不是「零」——公历年月日的规范写法就是这样，
/// 和「一百零五」里那个补位的「零」不是一回事。所以年份不能直接借 [`LOWER`]。
pub const YEAR_DIGITS: [&str; 10] = ["〇", "一", "二", "三", "四", "五", "六", "七", "八", "九"];

/// 日常读法
pub const LOWER: Style = Style {
    digits: ["零", "一", "二", "三", "四", "五", "六", "七", "八", "九"],
    units: ["", "十", "百", "千"],
    drop_one_before_ten: true,
};

/// 财务大写
pub const UPPER: Style = Style {
    digits: ["零", "壹", "贰", "叁", "肆", "伍", "陆", "柒", "捌", "玖"],
    units: ["", "拾", "佰", "仟"],
    drop_one_before_ten: false,
};

/// 一次「亿」跨几位。中文按 8 位切（万万为亿），不是 4 位。
const YI: usize = 8;

/// 支持的最大位数。u64 装得下 20 位，但 16 位之后的中文读法没人用，
/// 再长就原样返回——总比给出一个自己都念不顺的串好。
const MAX_DIGITS: usize = 16;

/// `123` → `一百二十三`
pub fn lower(digits: &str) -> String {
    LOWER.integer(digits)
}

/// `123` → `壹佰贰拾叁`
pub fn upper(digits: &str) -> String {
    UPPER.integer(digits)
}

/// `123` + `5` → `一百二十三点五`（小数部分逐位读，不按位值）
pub fn decimal_lower(integer: &str, fraction: &str) -> String {
    LOWER.decimal(integer, fraction)
}

/// `123` + `5` → `壹佰贰拾叁点伍`
pub fn decimal_upper(integer: &str, fraction: &str) -> String {
    UPPER.decimal(integer, fraction)
}

/// `123` + `5` → `一百二十三元五角`
pub fn amount_lower(integer: &str, fraction: &str) -> String {
    LOWER.amount(integer, fraction)
}

/// `123` + `05` → `壹佰贰拾叁元零伍分`
pub fn amount_upper(integer: &str, fraction: &str) -> String {
    UPPER.amount(integer, fraction)
}

impl Style {
    fn decimal(&self, integer: &str, fraction: &str) -> String {
        let mut out = self.integer(integer);
        out.push('点');
        // 小数逐位读：`0.05` 是「零点零五」，不是「零点五」
        for b in fraction.bytes() {
            out.push_str(self.digits[usize::from(b - b'0')]);
        }
        out
    }

    fn amount(&self, integer: &str, fraction: &str) -> String {
        let mut cents = fraction.bytes().map(|b| usize::from(b - b'0'));
        let jiao = cents.next().unwrap_or(0);
        let fen = cents.next().unwrap_or(0);
        // 「不足一元不写元」：`0.5` 是「五角」而不是「零元五角」
        let has_yuan = !integer.trim_start_matches('0').is_empty();

        let mut out = String::new();
        if has_yuan || (jiao == 0 && fen == 0) {
            out.push_str(&self.integer(integer));
            out.push('元');
        }
        if jiao == 0 && fen == 0 {
            out.push('整');
            return out;
        }
        if jiao > 0 {
            out.push_str(self.digits[jiao]);
            out.push('角');
        } else if has_yuan {
            // 有元无角有分：中间那个位是空的，读作「元零几分」
            out.push_str(self.digits[0]);
        }
        if fen > 0 {
            out.push_str(self.digits[fen]);
            out.push('分');
        }
        out
    }

    fn integer(&self, digits: &str) -> String {
        let digits = digits.trim_start_matches('0');
        if digits.is_empty() {
            return self.digits[0].to_owned();
        }
        if digits.len() > MAX_DIGITS || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return digits.to_owned();
        }
        let mut out = self.above_yi(digits);
        if self.drop_one_before_ten {
            let one_ten = format!("{}{}", self.digits[1], self.units[1]);
            if let Some(rest) = out.strip_prefix(one_ten.as_str()) {
                out = format!("{}{rest}", self.units[1]);
            }
        }
        out
    }

    /// 8 位以上：高段整体读完接「亿」，低 8 位非零时补上（不足 8 位补一个「零」）。
    fn above_yi(&self, digits: &str) -> String {
        if digits.len() <= YI {
            return self.below_yi(digits);
        }
        let cut = digits.len() - YI;
        let (high, low) = digits.split_at(cut);
        let mut out = self.above_yi(high);
        out.push('亿');
        let low = low.trim_start_matches('0');
        if !low.is_empty() {
            // 低位不足 8 位 = 高位末尾有零（`100000001` → 一亿零一）
            if low.len() < YI {
                out.push_str(self.digits[0]);
            }
            out.push_str(&self.below_yi(low));
        }
        out
    }

    /// 8 位以内：从右往左四位一节，节单位只有「万」。
    fn below_yi(&self, digits: &str) -> String {
        if digits.len() <= 4 {
            return self.section(digits);
        }
        let cut = digits.len() - 4;
        let (high, low) = digits.split_at(cut);
        let mut out = self.section(high);
        out.push('万');
        let low = low.trim_start_matches('0');
        if !low.is_empty() {
            // 同样：截短了说明高位末位是零，中间要补「零」
            if low.len() < 4 {
                out.push_str(self.digits[0]);
            }
            out.push_str(&self.section(low));
        }
        out
    }

    /// 四位以内。中间的零合并成一个，末尾的零不写。
    fn section(&self, digits: &str) -> String {
        let n = digits.len();
        let mut out = String::new();
        let mut pending_zero = false;
        for (i, b) in digits.bytes().enumerate() {
            let d = usize::from(b - b'0');
            if d == 0 {
                pending_zero = true;
                continue;
            }
            if pending_zero && !out.is_empty() {
                out.push_str(self.digits[0]);
            }
            pending_zero = false;
            out.push_str(self.digits[d]);
            out.push_str(self.units[n - 1 - i]);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lower_reads_the_standard_way() {
        assert_eq!(lower("0"), "零");
        assert_eq!(lower("7"), "七");
        assert_eq!(lower("10"), "十", "开头的「一十」省掉「一」");
        assert_eq!(lower("15"), "十五");
        assert_eq!(lower("20"), "二十");
        assert_eq!(lower("100"), "一百");
        assert_eq!(lower("105"), "一百零五", "中间的零只读一个");
        assert_eq!(lower("110"), "一百一十", "中间的不省「一」");
        assert_eq!(lower("1010"), "一千零一十");
        assert_eq!(lower("10005"), "一万零五", "四位空洞只补一个零");
        assert_eq!(lower("12000"), "一万二千");
    }

    #[test]
    fn zeros_between_and_after_sections() {
        // 节间补零；整节为零时什么都不补
        assert_eq!(lower("100000"), "十万");
        assert_eq!(lower("100001"), "十万零一");
        assert_eq!(lower("100200"), "十万零二百");
        assert_eq!(lower("20003000"), "二千万三千");
        assert_eq!(lower("20000300"), "二千万零三百");
        assert_eq!(lower("100100"), "十万零一百");
    }

    #[test]
    fn yi_is_eight_digits_not_four() {
        assert_eq!(lower("100000000"), "一亿");
        assert_eq!(lower("100000001"), "一亿零一");
        assert_eq!(lower("1500000000"), "十五亿");
        assert_eq!(lower("120000300"), "一亿二千万零三百");
        assert_eq!(
            lower("1234567890123"),
            "一万二千三百四十五亿六千七百八十九万零一百二十三"
        );
    }

    #[test]
    fn upper_keeps_the_leading_one() {
        assert_eq!(upper("15"), "壹拾伍");
        assert_eq!(upper("123"), "壹佰贰拾叁");
        assert_eq!(upper("10086"), "壹万零捌拾陆");
        assert_eq!(upper("0"), "零");
    }

    #[test]
    fn fractions_are_read_digit_by_digit() {
        assert_eq!(decimal_lower("123", "5"), "一百二十三点五");
        assert_eq!(decimal_upper("123", "5"), "壹佰贰拾叁点伍");
        assert_eq!(decimal_lower("0", "05"), "零点零五");
        assert_eq!(decimal_lower("10", "250"), "十点二五零");
        assert_eq!(decimal_upper("10", "25"), "壹拾点贰伍");
    }

    #[test]
    fn amounts_follow_the_invoice_style() {
        assert_eq!(amount_lower("123", "5"), "一百二十三元五角");
        assert_eq!(amount_upper("123", "5"), "壹佰贰拾叁元伍角");
        assert_eq!(amount_lower("123", ""), "一百二十三元整");
        assert_eq!(amount_upper("123", "00"), "壹佰贰拾叁元整");
        assert_eq!(amount_upper("123", "45"), "壹佰贰拾叁元肆角伍分");
        assert_eq!(amount_upper("123", "05"), "壹佰贰拾叁元零伍分");
        assert_eq!(amount_lower("10", "5"), "十元五角");
        assert_eq!(amount_upper("10", "5"), "壹拾元伍角");
        assert_eq!(amount_upper("100200", "3"), "壹拾万零贰佰元叁角");
        // 不足一元不写「元」
        assert_eq!(amount_lower("0", "5"), "五角");
        assert_eq!(amount_upper("0", "05"), "伍分");
        assert_eq!(amount_lower("0", ""), "零元整");
    }

    /// 超长或非数字：原样返回，不能给出念不顺的串，更不能 panic。
    #[test]
    fn out_of_range_is_returned_as_is() {
        assert_eq!(lower("12345678901234567"), "12345678901234567");
        assert_eq!(lower("12a"), "12a");
        assert_eq!(lower(""), "零");
        assert_eq!(lower("000"), "零");
    }
}
