use crate::domain::sheet::NamedColor;

use super::{CondOp, Condition, DateToken, Exponent, Fraction, Section, Token};

enum Tag {
    Color(NamedColor),
    /// Recognized but not rendered.
    StripOnly,
    /// `[$¥-411]`: the symbol becomes a literal.
    Currency(String),
    Condition(Condition),
    Unsupported,
}

/// `>=1000`, `<0`, `=5`, `<>0`.
fn parse_condition(tag: &str) -> Option<Condition> {
    let (op, rest) = if let Some(rest) = tag.strip_prefix("<>") {
        (CondOp::Ne, rest)
    } else if let Some(rest) = tag.strip_prefix("<=") {
        (CondOp::Le, rest)
    } else if let Some(rest) = tag.strip_prefix(">=") {
        (CondOp::Ge, rest)
    } else if let Some(rest) = tag.strip_prefix('<') {
        (CondOp::Lt, rest)
    } else if let Some(rest) = tag.strip_prefix('>') {
        (CondOp::Gt, rest)
    } else {
        (CondOp::Eq, tag.strip_prefix('=')?)
    };
    let value: f64 = rest.trim().parse().ok()?;
    value.is_finite().then_some(Condition { op, value })
}

/// The condition of a section, whether or not the rest of it parses.
pub(super) fn leading_condition(code: &str) -> Option<Condition> {
    let mut rest = code;
    while let Some(start) = rest.find('[') {
        let after = &rest[start + 1..];
        let end = after.find(']')?;
        if let Some(condition) = parse_condition(after[..end].trim()) {
            return Some(condition);
        }
        rest = &after[end + 1..];
    }
    None
}

fn classify_tag(tag: &str) -> Tag {
    if let Some(rest) = tag.strip_prefix('$') {
        let symbol = rest.split('-').next().unwrap_or("");
        return Tag::Currency(symbol.to_string());
    }
    if let Some(condition) = parse_condition(tag.trim()) {
        return Tag::Condition(condition);
    }
    let lowered = tag.to_ascii_lowercase();
    // [Color 5] etc.
    if let Some(rest) = lowered.strip_prefix("color") {
        if rest.trim().parse::<u8>().is_ok() {
            return Tag::StripOnly;
        }
    }
    match lowered.as_str() {
        "red" | "赤" => Tag::Color(NamedColor::Red),
        "blue" | "青" => Tag::Color(NamedColor::Blue),
        "green" | "緑" => Tag::Color(NamedColor::Green),
        "yellow" | "黄" => Tag::Color(NamedColor::Yellow),
        "magenta" | "紫" => Tag::Color(NamedColor::Magenta),
        "cyan" | "水" => Tag::Color(NamedColor::Cyan),
        "black" | "黒" => Tag::Color(NamedColor::Black),
        "white" | "白" => Tag::Color(NamedColor::White),
        _ => Tag::Unsupported, // conditions like [>=1000], ...
    }
}

fn end_cluster(in_number: &mut bool, number_done: &mut bool, scale: &mut u32, pending: &mut u32) {
    if *in_number {
        *in_number = false;
        *number_done = true;
        // commas not followed by digits are scalers
        *scale += *pending;
        *pending = 0;
    }
}

/// Stands in for the `?/?` part once `extract_fraction` has lifted it out of the code.
const FRACTION_MARK: char = '\u{1}';

fn is_placeholder(c: char) -> bool {
    matches!(c, '0' | '#' | '?')
}

/// `# ?/?` → the fraction spec and the code with the `?/?` part replaced by `FRACTION_MARK`.
/// `None` when the code has no placeholder run on both sides of a `/` outside quotes.
fn extract_fraction(code: &str) -> Option<(Fraction, String)> {
    let chars: Vec<char> = code.chars().collect();
    let mut in_quote = false;
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '"' => in_quote = !in_quote,
            '/' if !in_quote => {
                let start = chars[..i]
                    .iter()
                    .rposition(|c| !is_placeholder(*c))
                    .map_or(0, |p| p + 1);
                let after = &chars[i + 1..];
                let end = after
                    .iter()
                    .position(|c| !is_placeholder(*c) && !c.is_ascii_digit())
                    .unwrap_or(after.len());
                let numerator = &chars[start..i];
                let denominator = &after[..end];
                if numerator.is_empty() || denominator.is_empty() {
                    return None;
                }
                let fixed: String = denominator.iter().collect();
                // `0` is a placeholder first: `0/0` is a one-digit denominator, not zero
                let fraction = if denominator.iter().all(|c| is_placeholder(*c)) {
                    Fraction {
                        digits: denominator.len().min(3),
                        denominator: None,
                    }
                } else if denominator.iter().all(char::is_ascii_digit) {
                    let denominator = fixed.parse::<u64>().ok().filter(|d| *d > 0)?;
                    Fraction {
                        digits: fixed.len(),
                        denominator: Some(denominator),
                    }
                } else {
                    return None;
                };
                let mut rest: String = chars[..start].iter().collect();
                rest.push(FRACTION_MARK);
                rest.extend(after[end..].iter());
                return Some((fraction, rest));
            }
            _ => {}
        }
    }
    None
}

/// `None`: outside the subset.
pub(super) fn parse_section(code: &str) -> Option<Section> {
    let mut section = Section::default();
    let (fraction, code) = match extract_fraction(code) {
        Some((fraction, code)) => (Some(fraction), code),
        None => (None, code.to_string()),
    };
    let mut tokens = Vec::new();
    let mut literal = String::new();
    let mut chars = code.chars().peekable();
    let mut in_number = false;
    let mut in_frac = false;
    let mut number_done = false;
    let mut pending_commas = 0u32;

    let flush = |literal: &mut String, tokens: &mut Vec<Token>| {
        if !literal.is_empty() {
            tokens.push(Token::Literal(std::mem::take(literal)));
        }
    };

    while let Some(c) = chars.next() {
        match c {
            '[' => {
                let mut tag = String::new();
                for t in chars.by_ref() {
                    if t == ']' {
                        break;
                    }
                    tag.push(t);
                }
                if let Some(token) = elapsed_token(&tag) {
                    end_cluster(
                        &mut in_number,
                        &mut number_done,
                        &mut section.scale,
                        &mut pending_commas,
                    );
                    flush(&mut literal, &mut tokens);
                    tokens.push(Token::Date(token));
                    section.has_date = true;
                    continue;
                }
                match classify_tag(&tag) {
                    Tag::Color(color) => section.color = Some(color),
                    Tag::Condition(condition) => section.condition = Some(condition),
                    Tag::StripOnly => {}
                    Tag::Currency(symbol) => {
                        end_cluster(
                            &mut in_number,
                            &mut number_done,
                            &mut section.scale,
                            &mut pending_commas,
                        );
                        literal.push_str(&symbol);
                    }
                    Tag::Unsupported => return None,
                }
            }
            '"' => {
                end_cluster(
                    &mut in_number,
                    &mut number_done,
                    &mut section.scale,
                    &mut pending_commas,
                );
                for t in chars.by_ref() {
                    if t == '"' {
                        break;
                    }
                    literal.push(t);
                }
            }
            '\\' => {
                end_cluster(
                    &mut in_number,
                    &mut number_done,
                    &mut section.scale,
                    &mut pending_commas,
                );
                literal.push(chars.next()?);
            }
            // `_x` renders a space, `*x` is a fill
            '_' => {
                end_cluster(
                    &mut in_number,
                    &mut number_done,
                    &mut section.scale,
                    &mut pending_commas,
                );
                chars.next();
                literal.push(' ');
            }
            '*' => {
                end_cluster(
                    &mut in_number,
                    &mut number_done,
                    &mut section.scale,
                    &mut pending_commas,
                );
                chars.next();
            }
            '0' | '#' | '?' => {
                if number_done {
                    return None; // a second digit cluster is out of scope
                }
                if !in_number {
                    flush(&mut literal, &mut tokens);
                    tokens.push(Token::Number);
                    in_number = true;
                    section.has_number = true;
                }
                if pending_commas > 0 {
                    if in_frac {
                        section.scale += pending_commas; // commas after decimals scale
                    } else {
                        section.grouping = true; // commas *between* digits group
                    }
                    pending_commas = 0;
                }
                if in_frac {
                    section.max_frac += 1;
                    if c == '0' {
                        section.forced_frac = section.max_frac;
                    }
                } else {
                    section.int_places += 1;
                    if c == '0' {
                        section.min_int += 1;
                    }
                }
            }
            FRACTION_MARK => {
                end_cluster(
                    &mut in_number,
                    &mut number_done,
                    &mut section.scale,
                    &mut pending_commas,
                );
                flush(&mut literal, &mut tokens);
                tokens.push(Token::Fraction);
                section.fraction = fraction;
                section.has_number = true;
            }
            '.' if !in_frac
                && !number_done
                && (in_number || matches!(chars.peek(), Some('0') | Some('#') | Some('?'))) =>
            {
                if !in_number {
                    flush(&mut literal, &mut tokens);
                    tokens.push(Token::Number);
                    in_number = true;
                    section.has_number = true;
                }
                in_frac = true;
            }
            ',' if in_number => pending_commas += 1,
            '%' => {
                section.percent += 1;
                end_cluster(
                    &mut in_number,
                    &mut number_done,
                    &mut section.scale,
                    &mut pending_commas,
                );
                literal.push('%');
            }
            'E' | 'e' if matches!(chars.peek(), Some('+') | Some('-')) => {
                // the mantissa must come first; `E+` with nothing before it is a literal E
                if !in_number || in_frac && section.max_frac == 0 {
                    return None;
                }
                let plus = chars.next() == Some('+');
                let mut digits = 0;
                while chars.peek() == Some(&'0') {
                    chars.next();
                    digits += 1;
                }
                if digits == 0 {
                    return None;
                }
                section.exponent = Some(Exponent {
                    letter: c,
                    plus,
                    digits,
                });
                end_cluster(
                    &mut in_number,
                    &mut number_done,
                    &mut section.scale,
                    &mut pending_commas,
                );
            }
            '@' => {
                end_cluster(
                    &mut in_number,
                    &mut number_done,
                    &mut section.scale,
                    &mut pending_commas,
                );
                flush(&mut literal, &mut tokens);
                tokens.push(Token::Text);
                section.has_text = true;
            }
            'y' | 'Y' | 'm' | 'M' | 'd' | 'D' | 'h' | 'H' | 's' | 'S' | 'g' | 'G' | 'e' | 'E' => {
                // `General` must lex whole before its letters read as era codes
                if c.eq_ignore_ascii_case(&'g') {
                    let mut look = chars.clone();
                    let general = "eneral".chars().all(|want| {
                        look.next()
                            .is_some_and(|got| got.eq_ignore_ascii_case(&want))
                    });
                    if general {
                        for _ in 0..6 {
                            chars.next();
                        }
                        end_cluster(
                            &mut in_number,
                            &mut number_done,
                            &mut section.scale,
                            &mut pending_commas,
                        );
                        flush(&mut literal, &mut tokens);
                        tokens.push(Token::General);
                        section.has_general = true;
                        continue;
                    }
                }
                let mut run = 1;
                while chars.peek().is_some_and(|p| p.eq_ignore_ascii_case(&c)) {
                    chars.next();
                    run += 1;
                }
                let token = match c.to_ascii_lowercase() {
                    'y' => {
                        if run <= 2 {
                            DateToken::Year2
                        } else {
                            DateToken::Year4
                        }
                    }
                    'm' => match run {
                        1 => DateToken::MonthOrMinute { pad: false },
                        2 => DateToken::MonthOrMinute { pad: true },
                        3 => DateToken::MonthAbbr,
                        _ => DateToken::MonthFull,
                    },
                    'd' => match run {
                        1 => DateToken::Day { pad: false },
                        2 => DateToken::Day { pad: true },
                        3 => DateToken::WeekdayEnAbbr,
                        _ => DateToken::WeekdayEnFull,
                    },
                    'h' => DateToken::Hour { pad: run > 1 },
                    's' => DateToken::Second { pad: run > 1 },
                    'g' => match run {
                        1 => DateToken::EraLetter,
                        2 => DateToken::EraAbbr,
                        _ => DateToken::EraName,
                    },
                    _ => DateToken::EraYear { pad: run > 1 }, // 'e'
                };
                end_cluster(
                    &mut in_number,
                    &mut number_done,
                    &mut section.scale,
                    &mut pending_commas,
                );
                flush(&mut literal, &mut tokens);
                tokens.push(Token::Date(token));
                section.has_date = true;
            }
            'a' | 'A' => {
                end_cluster(
                    &mut in_number,
                    &mut number_done,
                    &mut section.scale,
                    &mut pending_commas,
                );
                let mut look = chars.clone();
                let ampm = ['m', '/', 'p', 'm'].iter().all(|&want| {
                    look.next()
                        .is_some_and(|got| got.eq_ignore_ascii_case(&want))
                });
                if ampm {
                    for _ in 0..4 {
                        chars.next();
                    }
                    flush(&mut literal, &mut tokens);
                    tokens.push(Token::Date(DateToken::AmPm));
                    section.has_date = true;
                    section.has_ampm = true;
                    continue;
                }
                // `A/P` is outside the subset; degrade rather than leak a literal
                let mut half = chars.clone();
                if half.next() == Some('/')
                    && half
                        .next()
                        .is_some_and(|got| got.eq_ignore_ascii_case(&'p'))
                {
                    return None;
                }
                let mut run = String::from(c);
                while let Some(&p) = chars.peek() {
                    if !p.eq_ignore_ascii_case(&'a') {
                        break;
                    }
                    chars.next();
                    run.push(p);
                }
                let token = match run.len() {
                    3 => Some(DateToken::WeekdayJaAbbr),
                    n if n >= 4 => Some(DateToken::WeekdayJaFull),
                    _ => None, // one or two bare `a`s stay literal
                };
                match token {
                    Some(token) => {
                        flush(&mut literal, &mut tokens);
                        tokens.push(Token::Date(token));
                        section.has_date = true;
                    }
                    None => literal.push_str(&run),
                }
            }
            other => {
                end_cluster(
                    &mut in_number,
                    &mut number_done,
                    &mut section.scale,
                    &mut pending_commas,
                );
                literal.push(other);
            }
        }
    }
    section.scale += pending_commas;
    // digits and date parts in one section are outside the subset
    if section.has_date && section.has_number {
        return None;
    }
    // text composes with literals only
    if section.has_text && (section.has_number || section.has_date || section.has_general) {
        return None;
    }
    // a fraction takes the whole number; decimals or an exponent next to it make no sense
    if section.fraction.is_some() && (section.max_frac > 0 || section.exponent.is_some()) {
        return None;
    }
    // `General` composes with nothing
    if section.has_general && (section.has_number || section.has_date) {
        return None;
    }
    // Excel's limit; beyond it 10^max_frac overflows f64 into NaN
    section.max_frac = section.max_frac.min(30);
    section.forced_frac = section.forced_frac.min(section.max_frac);
    flush(&mut literal, &mut tokens);
    resolve_minutes(&mut tokens);
    section.tokens = tokens;
    Some(section)
}

fn elapsed_token(tag: &str) -> Option<DateToken> {
    let all =
        |letter: char| !tag.is_empty() && tag.chars().all(|c| c.eq_ignore_ascii_case(&letter));
    let pad = tag.len() > 1;
    if all('h') {
        Some(DateToken::ElapsedHours { pad })
    } else if all('m') {
        Some(DateToken::ElapsedMinutes { pad })
    } else if all('s') {
        Some(DateToken::ElapsedSeconds { pad })
    } else {
        None
    }
}

/// `m` next to hours or seconds means minutes, otherwise months; literals are transparent to adjacency.
fn resolve_minutes(tokens: &mut [Token]) {
    let date_of = |t: &Token| match t {
        Token::Date(date) => Some(*date),
        _ => None,
    };
    for i in 0..tokens.len() {
        let Token::Date(DateToken::MonthOrMinute { pad }) = tokens[i] else {
            continue;
        };
        let prev = tokens[..i].iter().rev().find_map(date_of);
        let next = tokens[i + 1..].iter().find_map(date_of);
        let minutes = matches!(
            prev,
            Some(DateToken::Hour { .. } | DateToken::ElapsedHours { .. } | DateToken::AmPm)
        ) || matches!(
            next,
            Some(DateToken::Second { .. } | DateToken::ElapsedSeconds { .. })
        );
        tokens[i] = Token::Date(if minutes {
            DateToken::Minute { pad }
        } else {
            DateToken::Month { pad }
        });
    }
}
