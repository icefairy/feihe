//! 计算模式的候选生成：`/c` 后面那一串由它算。
//!
//! 同一个前缀按内容分派，用户只记一个键（`/c`）：
//!
//! ```text
//! /c123        → 一百二十三 / 壹佰贰拾叁 / 一百二十三元整 / 壹佰贰拾叁元整
//! /c123.5      → 一百二十三点五 / 壹佰贰拾叁点伍 / 一百二十三元五角 / 壹佰贰拾叁元伍角
//! /c1+2        → 3 / 1+2=3
//! /c1998-3-8   → 一九九八年三月八日 / 1998年3月8日 / 1998-03-08
//! ```
//!
//! **顺序要紧**：`1998` 和 `1998.3` 是整数与小数，先于日期判断；只有四位年份 + 合法
//! 月日（`1998-3-8`）才是日期，剩下的才当算式。`1-2-3` 不是日期（年份不是四位），
//! 照常算出 -4。
//!
//! 全部是纯函数，不读时钟、不碰全局状态，测试可以随便喂输入。

/// 算出 `/c` 后面那一串对应的候选。算不出来（语法错、除零）返回空。
pub fn forms(body: &str) -> Vec<String> {
    if body.is_empty() {
        return Vec::new();
    }
    if let Some(v) = number_forms(body) {
        return v;
    }
    if let Some(v) = date_forms(body) {
        return v;
    }
    match evaluate(body) {
        Some(result) => vec![result.clone(), format!("{body}={result}")],
        None => Vec::new(),
    }
}

/// 单个非负数：`123`、`123.5`。末尾刚敲了 `.`（`123.`）按整数给，候选不闪。
fn number_forms(body: &str) -> Option<Vec<String>> {
    let (integer, fraction) = body.split_once('.').unwrap_or((body, ""));
    let digit = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if integer.is_empty() || !digit(integer) || !digit(fraction) {
        return None;
    }
    // 一串数字超过 16 位没有中文读法（见 `number::MAX_DIGITS`），此时不冒充「金额大写」
    if integer.trim_start_matches('0').len() > 16 {
        return None;
    }
    let mut out = if fraction.is_empty() {
        vec![crate::number::lower(integer), crate::number::upper(integer)]
    } else {
        vec![
            crate::number::decimal_lower(integer, fraction),
            crate::number::decimal_upper(integer, fraction),
        ]
    };
    // 角分只有两位，三位以上没有票据写法
    if fraction.len() <= 2 {
        out.push(crate::number::amount_lower(integer, fraction));
        out.push(crate::number::amount_upper(integer, fraction));
    }
    Some(out)
}

/// `1998-3-8` / `1998/3/8` / `1998.3.8` → 中文日期、常见写法、补零的 ISO。
///
/// 只认**四位年份**：`98-3-8` 是 87（减法），不是 1998 年 3 月 8 日——年份写两位
/// 时的世纪猜不出来，猜错了比不支持更糟。
fn date_forms(body: &str) -> Option<Vec<String>> {
    let mut parts = body.split(['-', '/', '.']);
    let (y, m, d) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    if y.len() != 4 || !y.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let num = |s: &str| -> Option<u32> {
        if s.is_empty() || s.len() > 2 || !s.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        s.parse().ok()
    };
    let (month, day) = (num(m)?, num(d)?);
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(y.parse().ok()?, month) {
        return None;
    }
    let year: u32 = y.parse().ok()?;
    // 年份逐位读（一九九八），不是「一千九百九十八」；月日按读法（三月、二十五日）
    let year_cn: String = y
        .bytes()
        .map(|b| crate::number::YEAR_DIGITS[usize::from(b - b'0')])
        .collect();
    Some(vec![
        format!(
            "{year_cn}年{}月{}日",
            crate::number::lower(&month.to_string()),
            crate::number::lower(&day.to_string())
        ),
        format!("{year}年{month}月{day}日"),
        format!("{year:04}-{month:02}-{day:02}"),
    ])
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => 29,
        2 => 28,
        _ => 0,
    }
}

/// 算式求值：`+ - * / ^`、括号、小数、一元正负号。`x` 也当乘号（`*` 要按 Shift）。
///
/// 算不了返回 `None`——语法错、除零、结果不是有限数都算「算不了」。**不含**隐式乘法
/// （`2(3+4)` 不认）：输入法里少一个字符太常见，猜错了会给出一个看起来对的错答案。
pub fn evaluate(text: &str) -> Option<String> {
    let mut p = Parser {
        src: text.as_bytes(),
        at: 0,
    };
    let value = p.expr()?;
    if p.at != p.src.len() || !value.is_finite() {
        return None;
    }
    Some(format_number(value))
}

struct Parser<'a> {
    src: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.src.get(self.at).copied()
    }

    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        loop {
            match self.peek() {
                Some(b'+') => {
                    self.at += 1;
                    v += self.term()?;
                }
                Some(b'-') => {
                    self.at += 1;
                    v -= self.term()?;
                }
                _ => return Some(v),
            }
        }
    }

    fn term(&mut self) -> Option<f64> {
        let mut v = self.power()?;
        loop {
            match self.peek() {
                Some(b'*' | b'x') => {
                    self.at += 1;
                    v *= self.power()?;
                }
                Some(b'/') => {
                    self.at += 1;
                    let d = self.power()?;
                    if d == 0.0 {
                        return None;
                    }
                    v /= d;
                }
                _ => return Some(v),
            }
        }
    }

    /// 乘方右结合：`2^3^2` = 2^(3^2) = 512
    fn power(&mut self) -> Option<f64> {
        let base = self.unary()?;
        if self.peek() == Some(b'^') {
            self.at += 1;
            return Some(base.powf(self.power()?));
        }
        Some(base)
    }

    fn unary(&mut self) -> Option<f64> {
        match self.peek() {
            Some(b'-') => {
                self.at += 1;
                self.unary().map(|v| -v)
            }
            Some(b'+') => {
                self.at += 1;
                self.unary()
            }
            _ => self.atom(),
        }
    }

    fn atom(&mut self) -> Option<f64> {
        if self.peek() == Some(b'(') {
            self.at += 1;
            let v = self.expr()?;
            if self.peek() != Some(b')') {
                return None;
            }
            self.at += 1;
            return Some(v);
        }
        let start = self.at;
        while matches!(self.peek(), Some(b'0'..=b'9' | b'.')) {
            self.at += 1;
        }
        if start == self.at {
            return None;
        }
        std::str::from_utf8(&self.src[start..self.at])
            .ok()?
            .parse()
            .ok()
    }
}

/// 整数不带小数点；小数最多十位、去掉末尾的零。
///
/// 十位是刻意的：`0.1+0.2` 在 f64 里是 `0.30000000000000004`，保留十位后自然变成
/// `0.3`——用户要的是「算得对」，不是 IEEE 754 的真相。
fn format_number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        return format!("{v:.0}");
    }
    format!("{v:.10}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(body: &str) -> Vec<String> {
        forms(body)
    }

    #[test]
    fn arithmetic() {
        assert_eq!(f("1+2"), ["3", "1+2=3"]);
        assert_eq!(f("2*3+4")[0], "10");
        assert_eq!(f("2*(3+4)")[0], "14");
        assert_eq!(f("1/3")[0], "0.3333333333");
        assert_eq!(f("2^10")[0], "1024");
        assert_eq!(f("2^3^2")[0], "512", "乘方右结合");
        assert_eq!(f("-3+5")[0], "2");
        assert_eq!(f("0.1+0.2")[0], "0.3", "别把浮点误差漏给用户");
        assert_eq!(f("1.5x2")[0], "3", "x 也能当乘号");
    }

    #[test]
    fn bad_expressions_give_nothing() {
        assert!(f("1+").is_empty());
        assert!(f("1/0").is_empty());
        assert!(f("(1+2").is_empty());
        assert!(f("abc").is_empty());
        assert!(f("").is_empty());
        // 不做隐式乘法：`2(3+4)` 这种猜错了会给出看起来对的错答案
        assert!(f("2(3+4)").is_empty());
    }

    #[test]
    fn numbers_get_reading_and_amount() {
        assert_eq!(
            f("123"),
            ["一百二十三", "壹佰贰拾叁", "一百二十三元整", "壹佰贰拾叁元整"]
        );
        assert_eq!(
            f("123.5"),
            [
                "一百二十三点五",
                "壹佰贰拾叁点伍",
                "一百二十三元五角",
                "壹佰贰拾叁元伍角"
            ]
        );
        // 末尾刚敲点，候选不能闪
        assert_eq!(f("123."), f("123"));
        // 超过角分的小数只有读法
        assert_eq!(f("3.14159"), ["三点一四一五九", "叁点壹肆壹伍玖"]);
        assert_eq!(f("0"), ["零", "零", "零元整", "零元整"]);
    }

    #[test]
    fn dates_need_four_digit_years() {
        assert_eq!(
            f("1998-3-8"),
            ["一九九八年三月八日", "1998年3月8日", "1998-03-08"]
        );
        assert_eq!(f("1998/3/8")[0], "一九九八年三月八日");
        assert_eq!(f("2006.12.25")[0], "二〇〇六年十二月二十五日");
        assert_eq!(f("1998-03-08")[0], "一九九八年三月八日");
        // 年份不是四位 → 仍是算式
        assert_eq!(f("1-2-3")[0], "-4");
        assert_eq!(f("98-3-8")[0], "87");
        // 月日不合法 → 仍是算式
        assert_eq!(f("1998-2-30")[0], "1966");
        assert_eq!(f("1998-13-1")[0], "1984");
        // 段数不对
        assert_eq!(f("1998-3-8-1")[0], "1986");
        // 闰年
        assert_eq!(f("2024-2-29")[0], "二〇二四年二月二十九日");
        assert_eq!(f("2023-2-29")[0], "1992");
    }

    #[test]
    fn number_forms_win_over_dates_for_decimals() {
        // `1998.3` 是一个小数，不是「1998 年 3 月」
        assert_eq!(f("1998.3")[0], "一千九百九十八点三");
        assert_eq!(f("1998")[0], "一千九百九十八");
    }
}
