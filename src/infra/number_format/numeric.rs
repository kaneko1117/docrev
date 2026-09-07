use super::{Exponent, Fraction, Section, Token, general};

pub(super) fn render(section: &Section, value: f64, drop_sign: bool) -> String {
    let mut v = if drop_sign { value.abs() } else { value };
    for _ in 0..section.percent {
        v *= 100.0;
    }
    for _ in 0..section.scale {
        v /= 1000.0;
    }
    let auto_minus = v < 0.0;
    let (digits, exponent) = match section.exponent {
        Some(exponent) => scientific(v.abs(), section, exponent),
        None => (digit_string(v.abs(), section), String::new()),
    };
    let parts = section
        .fraction
        .map(|f| fraction_parts(v.abs(), section, f));

    let mut out = String::new();
    if auto_minus {
        out.push('-');
    }
    let mut tokens = section.tokens.iter().peekable();
    while let Some(token) = tokens.next() {
        match token {
            Token::Literal(text) => {
                // the gap between the whole and the fraction goes when either side is absent
                let gap = text.trim().is_empty()
                    && matches!(tokens.peek(), Some(Token::Fraction))
                    && parts
                        .as_ref()
                        .is_some_and(|p| p.fraction.is_none() || p.whole.is_empty());
                if !gap {
                    out.push_str(text);
                }
            }
            Token::Number => match &parts {
                Some(parts) => out.push_str(&parts.whole),
                None => {
                    out.push_str(&digits);
                    out.push_str(&exponent);
                }
            },
            Token::Fraction => {
                if let Some(text) = parts.as_ref().and_then(|p| p.fraction.as_deref()) {
                    out.push_str(text);
                }
            }
            Token::General => out.push_str(&general(v.abs())),
            // format() never sends a date or text section here
            Token::Date(_) | Token::Text => {}
        }
    }
    out
}

/// Mantissa digits and the `E±nn` tail; the exponent moves in steps of the integer placeholder
/// count, which makes `##0.0E+0` engineering notation.
fn scientific(abs: f64, section: &Section, exponent: Exponent) -> (String, String) {
    let step = section.int_places.max(1) as i32;
    let mut exp = if abs == 0.0 || !abs.is_finite() {
        0
    } else {
        abs.log10().floor() as i32
    };
    exp = exp.div_euclid(step) * step;
    let mut mantissa = abs / 10f64.powi(exp);
    let mut digits = digit_string(mantissa, section);
    // rounding can carry the mantissa past the step (9.999 -> 10.00); commas are not digits
    let int_len = digits
        .split('.')
        .next()
        .map_or(0, |s| s.chars().filter(char::is_ascii_digit).count());
    if int_len > step as usize {
        exp += step;
        mantissa = abs / 10f64.powi(exp);
        digits = digit_string(mantissa, section);
    }
    let sign = if exp < 0 {
        "-"
    } else if exponent.plus {
        "+"
    } else {
        ""
    };
    (
        digits,
        format!(
            "{}{sign}{:0>width$}",
            exponent.letter,
            exp.abs(),
            width = exponent.digits
        ),
    )
}

/// (whole part, `n/d`); a mixed number drops a zero whole part, and an exact whole drops the
/// fraction; an improper fraction has no whole part.
struct FractionParts {
    whole: String,
    fraction: Option<String>,
}

fn fraction_parts(abs: f64, section: &Section, spec: Fraction) -> FractionParts {
    let mixed = section.tokens.iter().any(|t| matches!(t, Token::Number));
    let abs = if abs.is_finite() { abs } else { 0.0 };
    let max_denominator = spec
        .denominator
        .unwrap_or_else(|| 10u64.pow(spec.digits as u32).saturating_sub(1))
        .max(1);
    let whole = abs.trunc();
    let (num, den) = match spec.denominator {
        Some(den) => (((abs - whole) * den as f64).round() as u64, den),
        None => closest_fraction(abs - whole, max_denominator),
    };
    // rounding up to a whole (0.999 with ?/? gives 1/1) folds into the whole part
    let (whole, num) = if num == den {
        (whole + 1.0, 0)
    } else {
        (whole, num)
    };
    if !mixed {
        // improper: the whole rides in the numerator; a free denominator collapses to 1
        let den = if num == 0 && spec.denominator.is_none() {
            1
        } else {
            den
        };
        let numerator = whole * den as f64 + num as f64;
        return FractionParts {
            whole: String::new(),
            fraction: Some(format!("{numerator:.0}/{den}")),
        };
    }
    // the whole part keeps the placeholders' padding and grouping
    let whole_text = if whole > 0.0 || num == 0 || section.min_int > 0 {
        digit_string(whole, section)
    } else {
        String::new()
    };
    FractionParts {
        whole: whole_text,
        fraction: (num > 0).then(|| format!("{num}/{den}")),
    }
}

/// The `n/d` with `d <= max` closest to `x` in `[0, 1]`.
fn closest_fraction(x: f64, max: u64) -> (u64, u64) {
    let mut best = (0u64, 1u64);
    let mut best_err = x;
    for den in 1..=max {
        let num = (x * den as f64).round() as u64;
        let err = (x - num as f64 / den as f64).abs();
        if err < best_err - 1e-12 {
            best = (num, den);
            best_err = err;
            if err == 0.0 {
                break;
            }
        }
    }
    best
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
