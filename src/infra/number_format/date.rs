//! Date/time rendering from adapter-resolved calendar parts.

use crate::infra::datetime::DateTimeParts;

use super::{DateToken, Section, Token};

const WEEKDAY_JA: [&str; 7] = ["日", "月", "火", "水", "木", "金", "土"];
const WEEKDAY_EN_ABBR: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const WEEKDAY_EN_FULL: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTH_EN_ABBR: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const MONTH_EN_FULL: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

fn push_padded(out: &mut String, value: u64, pad: bool) {
    if pad {
        out.push_str(&format!("{value:02}"));
    } else {
        out.push_str(&value.to_string());
    }
}

pub(super) fn render_datetime(section: &Section, parts: &DateTimeParts) -> String {
    // cells store whole seconds; rounding wards off f64 dust (13:05 is
    // 0.54513‥ whose ×86400 must land on 47100, not 47099.99‥)
    let total_seconds = (parts.serial * 86_400.0).round();
    let total = total_seconds.abs() as u64;
    let month_index = (parts.month as usize).clamp(1, 12) - 1;
    let weekday = parts.weekday_index();
    let (era, era_year) = parts.era();
    let hour12 = match parts.hour % 12 {
        0 => 12,
        hour => hour,
    };
    let has_elapsed = section.tokens.iter().any(|t| {
        matches!(
            t,
            Token::Date(
                DateToken::ElapsedHours { .. }
                    | DateToken::ElapsedMinutes { .. }
                    | DateToken::ElapsedSeconds { .. }
            )
        )
    });
    let mut out = String::new();
    if total_seconds < 0.0 && has_elapsed {
        out.push('-');
    }
    for token in &section.tokens {
        let date = match token {
            Token::Literal(text) => {
                out.push_str(text);
                continue;
            }
            // digit clusters never coexist with date tokens (parse rejects)
            Token::Number | Token::General => continue,
            Token::Date(date) => date,
        };
        match date {
            DateToken::Year4 => out.push_str(&parts.year.to_string()),
            DateToken::Year2 => push_padded(&mut out, u64::from(parts.year % 100), true),
            DateToken::Month { pad } | DateToken::MonthOrMinute { pad } => {
                push_padded(&mut out, u64::from(parts.month), *pad);
            }
            DateToken::MonthAbbr => out.push_str(MONTH_EN_ABBR[month_index]),
            DateToken::MonthFull => out.push_str(MONTH_EN_FULL[month_index]),
            DateToken::Day { pad } => push_padded(&mut out, u64::from(parts.day), *pad),
            DateToken::WeekdayEnAbbr => out.push_str(WEEKDAY_EN_ABBR[weekday]),
            DateToken::WeekdayEnFull => out.push_str(WEEKDAY_EN_FULL[weekday]),
            DateToken::WeekdayJaAbbr => out.push_str(WEEKDAY_JA[weekday]),
            DateToken::WeekdayJaFull => {
                out.push_str(WEEKDAY_JA[weekday]);
                out.push_str("曜日");
            }
            DateToken::Hour { pad } => {
                let hour = if section.has_ampm { hour12 } else { parts.hour };
                push_padded(&mut out, u64::from(hour), *pad);
            }
            // in elapsed sections the remainders come from the serial:
            // negative 1904-epoch durations saturate the calendar parts to 0
            DateToken::Minute { pad } => {
                let minute = if has_elapsed {
                    (total / 60) % 60
                } else {
                    u64::from(parts.minute)
                };
                push_padded(&mut out, minute, *pad);
            }
            DateToken::Second { pad } => {
                let second = if has_elapsed {
                    total % 60
                } else {
                    u64::from(parts.second)
                };
                push_padded(&mut out, second, *pad);
            }
            DateToken::AmPm => out.push_str(if parts.hour < 12 { "AM" } else { "PM" }),
            DateToken::EraLetter => out.push(era.letter),
            DateToken::EraAbbr => out.push_str(era.abbr),
            DateToken::EraName => out.push_str(era.name),
            DateToken::EraYear { pad } => push_padded(&mut out, u64::from(era_year), *pad),
            DateToken::ElapsedHours { pad } => push_padded(&mut out, total / 3600, *pad),
            DateToken::ElapsedMinutes { pad } => push_padded(&mut out, total / 60, *pad),
            DateToken::ElapsedSeconds { pad } => push_padded(&mut out, total, *pad),
        }
    }
    out
}
