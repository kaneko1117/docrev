//! Calendar components of a date/time cell. The adapter resolves the
//! workbook's serial number into these parts; this module derives only
//! weekday and era from them and owns no serial-to-calendar conversion.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateTimeKind {
    /// The value carries a calendar date.
    DateTime,
    /// A bare time of day — the serial has no date component.
    TimeOnly,
    /// An elapsed amount (`[h]`-style formats), not a point in time.
    Duration,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DateTimeParts {
    pub year: u16,
    /// 1-12.
    pub month: u8,
    /// 1-31.
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// Days since the Excel epoch; the fraction is the time of day.
    /// Elapsed-time tokens (`[h]`) count from this, not from the fields.
    pub serial: f64,
    pub kind: DateTimeKind,
}

pub(crate) struct Era {
    pub letter: char,
    pub abbr: &'static str,
    pub name: &'static str,
    start: (u16, u8, u8),
}

/// Newest first; the final entry is the floor (serials begin in 1900, well
/// inside Meiji, so anything earlier is a crafted input and clamps to it).
const ERAS: [Era; 5] = [
    Era {
        letter: 'R',
        abbr: "令",
        name: "令和",
        start: (2019, 5, 1),
    },
    Era {
        letter: 'H',
        abbr: "平",
        name: "平成",
        start: (1989, 1, 8),
    },
    Era {
        letter: 'S',
        abbr: "昭",
        name: "昭和",
        start: (1926, 12, 25),
    },
    Era {
        letter: 'T',
        abbr: "大",
        name: "大正",
        start: (1912, 7, 30),
    },
    Era {
        letter: 'M',
        abbr: "明",
        name: "明治",
        start: (1868, 1, 25),
    },
];

impl DateTimeParts {
    /// 0 = Sunday … 6 = Saturday (Sakamoto's method). Excel pretends the
    /// nonexistent 1900-02-29 existed, shifting weekdays before 1900-03-01;
    /// real calendars matter more than that two-month-old fiction, so we
    /// follow the actual weekday.
    pub fn weekday_index(&self) -> usize {
        const T: [i64; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
        let month = (self.month as usize).clamp(1, 12);
        let mut y = self.year as i64;
        if month < 3 {
            y -= 1;
        }
        let sum = y + y / 4 - y / 100 + y / 400 + T[month - 1] + self.day as i64;
        (sum.rem_euclid(7)) as usize
    }

    /// The rendering used when no supported format exists: date-bearing
    /// values keep `YYYY-MM-DD HH:MM:SS`, bare times drop the fictional
    /// epoch date, durations count elapsed time past 24 hours.
    pub fn fallback_text(&self) -> String {
        match self.kind {
            DateTimeKind::DateTime => format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                self.year, self.month, self.day, self.hour, self.minute, self.second
            ),
            DateTimeKind::TimeOnly => {
                format!("{:02}:{:02}:{:02}", self.hour, self.minute, self.second)
            }
            DateTimeKind::Duration => {
                let total = (self.serial * 86_400.0).round();
                let sign = if total < 0.0 { "-" } else { "" };
                let total = total.abs() as u64;
                format!(
                    "{sign}{}:{:02}:{:02}",
                    total / 3600,
                    (total / 60) % 60,
                    total % 60
                )
            }
        }
    }

    /// The Japanese era covering this date and the 1-based year within it.
    pub(crate) fn era(&self) -> (&'static Era, u16) {
        let key = (self.year, self.month, self.day);
        let era = ERAS
            .iter()
            .find(|e| key >= e.start)
            .unwrap_or(&ERAS[ERAS.len() - 1]);
        // saturation covers pre-Meiji clamps (0 + 1 = year 1); the +1 cannot
        // overflow because every era starts after 1868
        let year = self.year.saturating_sub(era.start.0) + 1;
        (era, year)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(year: u16, month: u8, day: u8) -> DateTimeParts {
        DateTimeParts {
            year,
            month,
            day,
            hour: 0,
            minute: 0,
            second: 0,
            serial: 0.0,
            kind: DateTimeKind::DateTime,
        }
    }

    #[test]
    fn weekdays_match_the_real_calendar() {
        assert_eq!(date(2026, 8, 31).weekday_index(), 1, "Monday");
        assert_eq!(date(2026, 9, 6).weekday_index(), 0, "Sunday");
        assert_eq!(date(2019, 5, 1).weekday_index(), 3, "Wednesday");
        assert_eq!(date(2000, 2, 29).weekday_index(), 2, "Tuesday");
        assert_eq!(date(1900, 3, 1).weekday_index(), 4, "Thursday");
    }

    #[test]
    fn hostile_month_values_do_not_panic() {
        assert!(date(2026, 0, 1).weekday_index() < 7);
        assert!(date(2026, 13, 1).weekday_index() < 7);
    }

    #[test]
    fn era_boundaries() {
        let era_of = |y, m, d| {
            let (era, year) = date(y, m, d).era();
            (era.name, year)
        };
        assert_eq!(era_of(2026, 8, 31), ("令和", 8));
        assert_eq!(era_of(2019, 5, 1), ("令和", 1));
        assert_eq!(era_of(2019, 4, 30), ("平成", 31));
        assert_eq!(era_of(1989, 1, 8), ("平成", 1));
        assert_eq!(era_of(1989, 1, 7), ("昭和", 64));
        assert_eq!(era_of(1926, 12, 25), ("昭和", 1));
        assert_eq!(era_of(1912, 7, 30), ("大正", 1));
        assert_eq!(era_of(1912, 7, 29), ("明治", 45));
        assert_eq!(era_of(1900, 1, 1), ("明治", 33));
    }

    #[test]
    fn dates_before_meiji_clamp_to_its_first_year() {
        let (era, year) = date(1867, 1, 1).era();
        assert_eq!(era.name, "明治");
        assert_eq!(year, 1);
    }

    #[test]
    fn era_at_the_numeric_ceiling_does_not_overflow() {
        let (era, year) = date(u16::MAX, 12, 31).era();
        assert_eq!(era.name, "令和");
        assert_eq!(year, 63517);
    }

    #[test]
    fn fallback_text_never_shows_a_fictional_date() {
        let full = DateTimeParts {
            hour: 13,
            minute: 5,
            second: 0,
            ..date(2026, 8, 31)
        };
        assert_eq!(full.fallback_text(), "2026-08-31 13:05:00");

        let time_only = DateTimeParts {
            hour: 13,
            minute: 5,
            second: 0,
            kind: DateTimeKind::TimeOnly,
            ..date(1899, 12, 31)
        };
        assert_eq!(time_only.fallback_text(), "13:05:00");

        let elapsed = DateTimeParts {
            serial: 1.5,
            kind: DateTimeKind::Duration,
            ..date(1900, 1, 1)
        };
        assert_eq!(elapsed.fallback_text(), "36:00:00");
    }
}
