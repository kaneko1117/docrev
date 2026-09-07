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
use section::{leading_condition, parse_section};

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
    Sections(Vec<Slot>),
}

/// One `;`-separated section; `section` is `None` outside the subset, and `condition` survives
/// that so the code still selects by condition.
#[derive(Debug, Clone, PartialEq)]
struct Slot {
    condition: Option<Condition>,
    section: Option<Section>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum CondOp {
    Lt,
    Le,
    Eq,
    Ne,
    Ge,
    Gt,
}

/// `[>=1000]`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Condition {
    op: CondOp,
    value: f64,
}

impl Condition {
    fn holds(&self, value: f64) -> bool {
        match self.op {
            CondOp::Lt => value < self.value,
            CondOp::Le => value <= self.value,
            CondOp::Eq => value == self.value,
            CondOp::Ne => value != self.value,
            CondOp::Ge => value >= self.value,
            CondOp::Gt => value > self.value,
        }
    }
}

/// `E+00`: `letter` keeps the case, `plus` writes the sign of a positive exponent too,
/// `digits` pads it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Exponent {
    letter: char,
    plus: bool,
    digits: usize,
}

/// `?/?`, `??/??` or `?/8`: `denominator` is the fixed one, else the largest with `digits` digits.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Fraction {
    digits: usize,
    denominator: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Default)]
struct Section {
    color: Option<NamedColor>,
    tokens: Vec<Token>,
    grouping: bool,
    min_int: usize,
    /// Every integer placeholder, `#` and `?` included; sets the exponent step.
    int_places: usize,
    forced_frac: usize,
    max_frac: usize,
    condition: Option<Condition>,
    exponent: Option<Exponent>,
    fraction: Option<Fraction>,
    has_text: bool,
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
    /// The `?/?` part; the integer part, if any, is a `Number` before it.
    Fraction,
    /// `@`: the cell's text.
    Text,
}

impl Section {
    fn renders(&self) -> bool {
        self.has_number || self.has_date || self.has_general || self.has_text
    }
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
        let sections: Vec<Slot> = split_sections(trimmed)
            .into_iter()
            .take(4)
            .map(|part| {
                let section = parse_section(&part);
                Slot {
                    condition: section
                        .as_ref()
                        .and_then(|s| s.condition)
                        .or_else(|| leading_condition(&part)),
                    section,
                }
            })
            .collect();
        // at least one section must render something
        if !sections
            .iter()
            .filter_map(|s| s.section.as_ref())
            .any(Section::renders)
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

    /// Slots numbers use, in order; the text section is left out.
    fn numeric_slots(&self) -> Vec<&Slot> {
        match &self.kind {
            Kind::General => Vec::new(),
            Kind::Sections(sections) => sections
                .iter()
                .filter(|s| !s.section.as_ref().is_some_and(|s| s.has_text))
                .collect(),
        }
    }

    fn first_numeric(&self) -> Option<&Section> {
        self.numeric_slots()
            .first()
            .and_then(|s| s.section.as_ref())
    }

    fn text_section(&self) -> Option<&Section> {
        match &self.kind {
            Kind::General => None,
            Kind::Sections(sections) => sections
                .iter()
                .filter_map(|s| s.section.as_ref())
                .find(|s| s.has_text),
        }
    }

    /// (section, drop the sign); `None` when the value lands on a section outside the subset.
    /// With conditions the first holding one wins, then the first unconditional; without them
    /// the second section takes negatives and the third zero.
    fn pick(&self, value: f64) -> Option<(&Section, bool)> {
        let slots = self.numeric_slots();
        if slots.iter().any(|s| s.condition.is_some()) {
            let matched = slots
                .iter()
                .find(|s| s.condition.is_some_and(|c| c.holds(value)));
            let otherwise = slots.iter().find(|s| s.condition.is_none());
            return matched
                .or(otherwise)
                .and_then(|s| s.section.as_ref())
                .map(|s| (s, false));
        }
        let (slot, drop_sign) = if value < 0.0 && slots.len() > 1 {
            (slots[1], true)
        } else if value == 0.0 && slots.len() > 2 {
            (slots[2], false)
        } else {
            (*slots.first()?, false)
        };
        slot.section.as_ref().map(|s| (s, drop_sign))
    }

    pub fn format(&self, value: f64) -> Formatted {
        let raw = Formatted {
            text: general(value),
            color: None,
        };
        let Some((section, drop_sign)) = self.pick(value) else {
            return raw;
        };
        if section.has_date {
            // no calendar parts here
            return raw;
        }
        Formatted {
            text: render(section, value, drop_sign),
            color: section.color,
        }
    }

    /// `None` when the code has no text section, which leaves text cells as they are.
    pub fn format_text(&self, text: &str) -> Option<Formatted> {
        let section = self.text_section()?;
        let mut out = String::new();
        for token in &section.tokens {
            match token {
                Token::Literal(literal) => out.push_str(literal),
                Token::Text => out.push_str(text),
                _ => {}
            }
        }
        Some(Formatted {
            text: out,
            color: section.color,
        })
    }

    /// Only a text section: numbers under it read as text, as in Excel.
    pub fn is_text_only(&self) -> bool {
        self.text_section().is_some() && self.numeric_slots().is_empty()
    }

    /// A number under a text-only format, composed like text.
    pub fn format_number_as_text(&self, value: f64) -> Option<Formatted> {
        self.format_text(&general(value))
    }

    pub fn is_date(&self) -> bool {
        self.first_numeric().is_some_and(|s| s.has_date)
    }

    /// Always the first section; a numeric format paints the serial. `None` when that section
    /// is outside the subset, so the caller keeps its own text.
    pub fn format_datetime(&self, parts: &DateTimeParts) -> Option<Formatted> {
        if self.is_general() {
            return Some(Formatted {
                text: general(parts.serial),
                color: None,
            });
        }
        let first = self.first_numeric()?;
        Some(if first.has_date {
            Formatted {
                text: render_datetime(first, parts),
                color: first.color,
            }
        } else {
            self.format(parts.serial)
        })
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
