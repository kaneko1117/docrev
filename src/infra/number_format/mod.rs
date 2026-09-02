//! Excel number formats (ECMA-376 §18.8.30); anything outside the subset degrades to `General`.

use crate::domain::sheet::NamedColor;
use crate::infra::datetime::DateTimeParts;

mod date;
mod numeric;
mod section;
#[cfg(test)]
mod tests;

use date::render_datetime;
use numeric::render;
use section::parse_section;

#[derive(Debug, Clone, PartialEq)]
pub struct Formatted {
    pub text: String,
    pub color: Option<NamedColor>,
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
    color: Option<NamedColor>,
    tokens: Vec<Token>,
    grouping: bool,
    min_int: usize,
    forced_frac: usize,
    max_frac: usize,
    /// `%` count; each one multiplies by 100.
    percent: u32,
    /// Trailing comma count; each one divides by 1000.
    scale: u32,
    has_number: bool,
    has_date: bool,
    /// Hours render on the 12-hour clock.
    has_ampm: bool,
    has_general: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Literal(String),
    Number,
    /// Renders the raw value; lexed as one token so its letters never read as date codes.
    General,
    Date(DateToken),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum DateToken {
    Year4,
    Year2,
    Month {
        pad: bool,
    },
    /// An `m` run before the minute-vs-month resolution.
    MonthOrMinute {
        pad: bool,
    },
    MonthAbbr,
    MonthFull,
    Day {
        pad: bool,
    },
    WeekdayEnAbbr,
    WeekdayEnFull,
    WeekdayJaAbbr,
    WeekdayJaFull,
    Hour {
        pad: bool,
    },
    Minute {
        pad: bool,
    },
    Second {
        pad: bool,
    },
    AmPm,
    EraLetter,
    EraAbbr,
    EraName,
    EraYear {
        pad: bool,
    },
    ElapsedHours {
        pad: bool,
    },
    ElapsedMinutes {
        pad: bool,
    },
    ElapsedSeconds {
        pad: bool,
    },
}

/// Excel caps codes at 255; a long digit run would make padding quadratic.
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
        let mut parts = split_sections(trimmed);
        // a trailing lone `@` is the text section, which numbers never use
        if parts.len() > 1 && parts.last().map(String::as_str).map(str::trim) == Some("@") {
            parts.pop();
        }
        let mut sections = Vec::new();
        // pos; neg; zero
        for part in parts.into_iter().take(3) {
            match parse_section(&part) {
                Some(section) => sections.push(section),
                None => {
                    return Self {
                        kind: Kind::General,
                    };
                }
            }
        }
        // at least one section must render digits or date parts
        if !sections
            .iter()
            .any(|s| s.has_number || s.has_date || s.has_general)
        {
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
        if section.has_date {
            // no calendar parts here
            return Formatted {
                text: general(value),
                color: None,
            };
        }
        Formatted {
            text: render(section, value, drop_sign),
            color: section.color,
        }
    }

    pub fn is_date(&self) -> bool {
        matches!(&self.kind, Kind::Sections(sections)
            if sections.first().is_some_and(|s| s.has_date))
    }

    /// Always the first section; a numeric format paints the serial.
    pub fn format_datetime(&self, parts: &DateTimeParts) -> Formatted {
        let Kind::Sections(sections) = &self.kind else {
            return Formatted {
                text: general(parts.serial),
                color: None,
            };
        };
        match sections.first() {
            Some(first) if first.has_date => Formatted {
                text: render_datetime(first, parts),
                color: first.color,
            },
            Some(_) => self.format(parts.serial),
            None => Formatted {
                text: general(parts.serial),
                color: None,
            },
        }
    }
}

fn general(value: f64) -> String {
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
