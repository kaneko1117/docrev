//! Numeric rendering: digit clusters, grouping, scaling, padding.

use super::{Section, Token};

pub(super) fn render(section: &Section, value: f64, drop_sign: bool) -> String {
    let mut v = if drop_sign { value.abs() } else { value };
    for _ in 0..section.percent {
        v *= 100.0;
    }
    for _ in 0..section.scale {
        v /= 1000.0;
    }
    let auto_minus = v < 0.0;
    let digits = digit_string(v.abs(), section);

    let mut out = String::new();
    if auto_minus {
        out.push('-');
    }
    for token in &section.tokens {
        match token {
            Token::Literal(text) => out.push_str(text),
            Token::Number => out.push_str(&digits),
            // date sections never reach here (format() showed the raw value)
            Token::Date(_) => {}
        }
    }
    out
}

fn digit_string(abs: f64, section: &Section) -> String {
    // Excel rounds half away from zero; f64 formatting rounds half to even
    let factor = 10f64.powi(section.max_frac as i32);
    let abs = (abs * factor).round() / factor;
    let rounded = format!("{:.*}", section.max_frac, abs);
    let (int_part, frac_part) = rounded.split_once('.').unwrap_or((rounded.as_str(), ""));

    let mut int_digits = int_part.to_string();
    if section.min_int == 0 && int_digits == "0" && section.max_frac > 0 {
        int_digits.clear(); // "#.00" / ".00" show no bare integer zero
    }
    while int_digits.len() < section.min_int {
        int_digits.insert(0, '0');
    }
    if section.grouping {
        int_digits = group_thousands(&int_digits);
    }

    let mut frac = frac_part.to_string();
    while frac.len() > section.forced_frac && frac.ends_with('0') {
        frac.pop();
    }

    if frac.is_empty() {
        int_digits
    } else {
        format!("{int_digits}.{frac}")
    }
}

fn group_thousands(digits: &str) -> String {
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    let count = digits.chars().count();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (count - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}
