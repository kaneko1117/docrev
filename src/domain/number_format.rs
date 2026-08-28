//! Excel number-format engine — the practical subset. Anything the
//! parser does not understand degrades to `General` (the raw value).

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FormatColor {
    Red,
    Blue,
    Green,
    Yellow,
    Magenta,
    Cyan,
    Black,
    White,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Formatted {
    pub text: String,
    pub color: Option<FormatColor>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NumberFormat {
    kind: Kind,
}

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    General,
    Sections(Vec<Section>),
}

#[derive(Debug, Clone, PartialEq, Default)]
struct Section {
    color: Option<FormatColor>,
    tokens: Vec<Token>,
    grouping: bool,
    min_int: usize,
    forced_frac: usize,
    max_frac: usize,
    /// `%` count — each one multiplies by 100.
    percent: u32,
    /// Trailing commas: each one divides by 1000 (`#,##0,` = thousands).
    scale: u32,
    has_number: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Literal(String),
    Number,
}

/// Excel itself caps custom format codes at 255 characters; far beyond that
/// the code is corrupt or hostile (format codes come from untrusted files,
/// and a multi-megabyte digit run would make padding quadratic).
const MAX_CODE_LEN: usize = 512;

impl NumberFormat {
    pub fn parse(code: &str) -> Self {
        let trimmed = code.trim();
        if trimmed.is_empty()
            || trimmed.len() > MAX_CODE_LEN
            || trimmed.eq_ignore_ascii_case("general")
        {
            return Self {
                kind: Kind::General,
            };
        }
        let mut sections = Vec::new();
        // only the first three sections matter (pos; neg; zero)
        for part in split_sections(trimmed).into_iter().take(3) {
            match parse_section(&part) {
                Some(section) => sections.push(section),
                None => {
                    return Self {
                        kind: Kind::General,
                    };
                }
            }
        }
        // literal-only sections (a zero shown as "-") are fine, but at least
        // one section must actually render digits
        if !sections.iter().any(|s| s.has_number) {
            return Self {
                kind: Kind::General,
            };
        }
        Self {
            kind: Kind::Sections(sections),
        }
    }

    pub fn is_general(&self) -> bool {
        self.kind == Kind::General
    }

    pub fn format(&self, value: f64) -> Formatted {
        let Kind::Sections(sections) = &self.kind else {
            return Formatted {
                text: general(value),
                color: None,
            };
        };
        let Some(first) = sections.first() else {
            return Formatted {
                text: general(value),
                color: None,
            };
        };
        let (section, drop_sign) = if value < 0.0 {
            match sections.get(1) {
                Some(negative) => (negative, true),
                None => (first, false),
            }
        } else if value == 0.0 {
            (sections.get(2).unwrap_or(first), false)
        } else {
            (first, false)
        };
        Formatted {
            text: render(section, value, drop_sign),
            color: section.color,
        }
    }
}

/// Excel's General rendering of a bare number — the fallback for cells with
/// no (or an unsupported) format.
pub(crate) fn general(value: f64) -> String {
    value.to_string()
}

/// Split on `;` outside quoted literals.
fn split_sections(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    for c in code.chars() {
        match c {
            '"' => {
                in_quote = !in_quote;
                current.push(c);
            }
            ';' if !in_quote => out.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    out.push(current);
    out
}

enum Tag {
    Color(FormatColor),
    /// Recognized but not rendered — stripped from the output.
    StripOnly,
    /// `[$¥-411]` locale currency: the symbol becomes a literal.
    Currency(String),
    Unsupported,
}

fn classify_tag(tag: &str) -> Tag {
    if let Some(rest) = tag.strip_prefix('$') {
        let symbol = rest.split('-').next().unwrap_or("");
        return Tag::Currency(symbol.to_string());
    }
    let lowered = tag.to_ascii_lowercase();
    // indexed palette colors ([Color 5] etc.) — recognized, not rendered
    if let Some(rest) = lowered.strip_prefix("color") {
        if rest.trim().parse::<u8>().is_ok() {
            return Tag::StripOnly;
        }
    }
    match lowered.as_str() {
        "red" | "赤" => Tag::Color(FormatColor::Red),
        "blue" | "青" => Tag::Color(FormatColor::Blue),
        "green" | "緑" => Tag::Color(FormatColor::Green),
        "yellow" | "黄" => Tag::Color(FormatColor::Yellow),
        "magenta" | "紫" => Tag::Color(FormatColor::Magenta),
        "cyan" | "水" => Tag::Color(FormatColor::Cyan),
        "black" | "黒" => Tag::Color(FormatColor::Black),
        "white" | "白" => Tag::Color(FormatColor::White),
        _ => Tag::Unsupported, // conditions like [>=1000], ...
    }
}

fn end_cluster(in_number: &mut bool, number_done: &mut bool, scale: &mut u32, pending: &mut u32) {
    if *in_number {
        *in_number = false;
        *number_done = true;
        // commas not followed by more digits were trailing scalers
        *scale += *pending;
        *pending = 0;
    }
}

/// `None` = the section uses something outside our subset.
fn parse_section(code: &str) -> Option<Section> {
    let mut section = Section::default();
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
                match classify_tag(&tag) {
                    Tag::Color(color) => section.color = Some(color),
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
            // `_x` reserves the width of x (render a space), `*x` is a fill
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
                } else if c == '0' {
                    section.min_int += 1;
                }
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
            'E' | 'e' if matches!(chars.peek(), Some('+') | Some('-')) => return None,
            '/' if section.has_number => return None, // fractions
            '@' => return None,                       // text composition
            'y' | 'Y' | 'd' | 'D' | 'h' | 'H' | 's' | 'S' | 'm' | 'M' => return None, // date codes
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
    // Excel renders at most 30 decimal places, and far beyond that the
    // rounding scale (10^max_frac) overflows f64 into NaN output
    section.max_frac = section.max_frac.min(30);
    section.forced_frac = section.forced_frac.min(section.max_frac);
    flush(&mut literal, &mut tokens);
    section.tokens = tokens;
    Some(section)
}

fn render(section: &Section, value: f64, drop_sign: bool) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(code: &str, value: f64) -> String {
        NumberFormat::parse(code).format(value).text
    }

    #[test]
    fn plain_placeholders() {
        assert_eq!(fmt("0", 5.0), "5");
        assert_eq!(fmt("0", -5.0), "-5");
        assert_eq!(fmt("0", 5.6), "6");
        assert_eq!(fmt("0.00", 12.5), "12.50");
        assert_eq!(fmt("0.0#", 1.25), "1.25");
        assert_eq!(fmt("0.0#", 1.2), "1.2");
        assert_eq!(fmt("0000", 42.0), "0042");
    }

    #[test]
    fn thousands_grouping() {
        assert_eq!(fmt("#,##0", 1234567.0), "1,234,567");
        assert_eq!(fmt("#,##0", 123.0), "123");
        assert_eq!(fmt("#,##0.00", 1234.5), "1,234.50");
    }

    #[test]
    fn percent_scaling() {
        assert_eq!(fmt("0%", 0.15), "15%");
        assert_eq!(fmt("0.0%", 0.1234), "12.3%");
    }

    #[test]
    fn currency_literals() {
        assert_eq!(fmt("¥#,##0", 1200.0), "¥1,200");
        assert_eq!(fmt("$#,##0.00", 12.5), "$12.50");
        assert_eq!(fmt("¥#,##0", -1200.0), "-¥1,200");
        // locale currency tag
        assert_eq!(fmt("[$¥-411]#,##0", 1200.0), "¥1,200");
    }

    #[test]
    fn negative_and_zero_sections() {
        let code = "#,##0;▲#,##0;\"-\"";
        assert_eq!(fmt(code, 1234.0), "1,234");
        assert_eq!(fmt(code, -1234.0), "▲1,234");
        assert_eq!(fmt(code, 0.0), "-");
        // two sections: negatives drop the automatic minus
        assert_eq!(fmt("0;(0)", -5.0), "(5)");
    }

    #[test]
    fn red_negative_carries_the_color() {
        let format = NumberFormat::parse("#,##0;[赤]▲#,##0");
        assert_eq!(format.format(-1234.0).color, Some(FormatColor::Red));
        assert_eq!(format.format(-1234.0).text, "▲1,234");
        assert_eq!(format.format(1234.0).color, None);

        let english = NumberFormat::parse("#,##0;[Red]-#,##0");
        assert_eq!(english.format(-1.0).color, Some(FormatColor::Red));
    }

    #[test]
    fn all_eight_standard_colors_are_recognized() {
        for (tag, expected) in [
            ("Blue", FormatColor::Blue),
            ("緑", FormatColor::Green),
            ("Yellow", FormatColor::Yellow),
            ("紫", FormatColor::Magenta),
            ("Cyan", FormatColor::Cyan),
            ("黒", FormatColor::Black),
            ("White", FormatColor::White),
        ] {
            let format = NumberFormat::parse(&format!("[{tag}]0"));
            assert_eq!(format.format(5.0).color, Some(expected), "{tag}");
            assert_eq!(format.format(5.0).text, "5");
        }
        // indexed palette colors are stripped, not rendered — and must not
        // knock the whole format back to General
        let indexed = NumberFormat::parse("[Color 5]#,##0");
        assert!(!indexed.is_general());
        assert_eq!(indexed.format(1234.0).text, "1,234");
        assert_eq!(indexed.format(1234.0).color, None);
    }

    #[test]
    fn trailing_comma_scales_to_thousands() {
        assert_eq!(fmt("#,##0,\"千円\"", 1234000.0), "1,234千円");
        assert_eq!(fmt("0,,", 2500000.0), "3"); // millions, rounded
    }

    #[test]
    fn accounting_style_padding() {
        assert_eq!(fmt("_(#,##0_)", 1234.0), " 1,234 ");
        assert_eq!(fmt("_-¥* #,##0_-", 1200.0), " ¥1,200 "); // *-fills are dropped
    }

    #[test]
    fn decimal_then_trailing_comma_scales() {
        assert_eq!(fmt("#,##0.0,", 1234500.0), "1,234.5");
    }

    #[test]
    fn literal_between_digit_clusters_falls_back() {
        assert!(NumberFormat::parse("0\"個\"0").is_general());
        assert_eq!(fmt("#,##0\"円\"", 1234.0), "1,234円");
    }

    #[test]
    fn fraction_only_formats() {
        assert_eq!(fmt(".00", 0.5), ".50");
        assert_eq!(fmt(".0", 0.25), ".3");
    }

    #[test]
    fn double_percent_compounds() {
        assert_eq!(fmt("0%%", 0.0015), "15%%");
        assert_eq!(fmt("0.0%", 0.1234), "12.3%");
    }

    #[test]
    fn unsupported_codes_fall_back_to_general() {
        for code in [
            "0.00E+00",    // scientific
            "# ?/?",       // fractions
            "[>=1000]0;0", // conditions
            "yyyy/mm/dd",  // dates
            "@",           // text section
            "0;0;0;@",     // has an unsupported 4th... first three are fine
        ] {
            let format = NumberFormat::parse(code);
            if code == "0;0;0;@" {
                // the 4th (text) section is simply ignored
                assert!(!format.is_general(), "{code}");
                continue;
            }
            assert!(format.is_general(), "{code} should fall back");
            assert_eq!(format.format(1.5).text, "1.5");
        }
    }

    #[test]
    fn general_matches_the_raw_rendering() {
        let format = NumberFormat::parse("General");
        assert_eq!(format.format(120.0).text, "120");
        assert_eq!(format.format(80.5).text, "80.5");
    }

    #[test]
    fn absurdly_long_codes_degrade_to_general_instantly() {
        // a hostile styles.xml can carry a multi-megabyte digit run; padding
        // to that many placeholders would be quadratic, so it must not parse
        let bomb = "0".repeat(5_000_000);
        let format = NumberFormat::parse(&bomb);
        assert!(format.is_general());
        assert_eq!(format.format(1.5).text, "1.5");

        let frac_bomb = format!("0.{}", "0".repeat(5_000_000));
        assert!(NumberFormat::parse(&frac_bomb).is_general());
    }

    #[test]
    fn the_longest_accepted_code_still_formats_in_bounded_time() {
        // MAX_CODE_LEN caps digit placeholders at 512, so the quadratic
        // zero-padding worst case is ~512² byte moves — trivial. This pins
        // the bound: rendering (not just parsing) must stay instant.
        let widest = "0".repeat(512);
        let format = NumberFormat::parse(&widest);
        assert!(!format.is_general(), "512 chars is within the cap");
        let text = format.format(7.0).text;
        assert_eq!(text.len(), 512);
        assert!(text.ends_with('7'));
    }

    #[test]
    fn decimal_places_clamp_at_excels_thirty() {
        // 10^510 overflows f64 — without the clamp this rendered "NaN"
        let deepest = format!("0.{}", "0".repeat(510));
        let text = NumberFormat::parse(&deepest).format(1.5).text;
        assert!(text.starts_with("1.5"), "got {text}");
        assert_eq!(text.len(), 2 + 30, "padding stops at 30 decimals");

        assert_eq!(NumberFormat::parse("0.00").format(1.5).text, "1.50");
    }
}
