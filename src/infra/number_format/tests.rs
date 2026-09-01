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
    assert_eq!(format.format(-1234.0).color, Some(NamedColor::Red));
    assert_eq!(format.format(-1234.0).text, "▲1,234");
    assert_eq!(format.format(1234.0).color, None);

    let english = NumberFormat::parse("#,##0;[Red]-#,##0");
    assert_eq!(english.format(-1.0).color, Some(NamedColor::Red));
}

#[test]
fn all_eight_standard_colors_are_recognized() {
    for (tag, expected) in [
        ("Blue", NamedColor::Blue),
        ("緑", NamedColor::Green),
        ("Yellow", NamedColor::Yellow),
        ("紫", NamedColor::Magenta),
        ("Cyan", NamedColor::Cyan),
        ("黒", NamedColor::Black),
        ("White", NamedColor::White),
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
        "0.00E+00",      // scientific
        "# ?/?",         // fractions
        "[>=1000]0;0",   // conditions
        "mm:ss.00",      // fractional seconds mix digits into a date
        "0.0\"日\"yyyy", // digits and date parts in one section
        "[DBNum1]yyyy",  // kanji numerals
        "@",             // text section
        "0;0;0;@",       // has an unsupported 4th... first three are fine
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

use crate::infra::datetime::{DateTimeKind, DateTimeParts};

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

fn time(hour: u8, minute: u8, second: u8) -> DateTimeParts {
    let seconds = u32::from(hour) * 3600 + u32::from(minute) * 60 + u32::from(second);
    DateTimeParts {
        hour,
        minute,
        second,
        serial: f64::from(seconds) / 86_400.0,
        kind: DateTimeKind::TimeOnly,
        ..date(1899, 12, 31)
    }
}

/// Mirrors the adapter: clock fields are the serial's remainder.
fn duration(serial: f64) -> DateTimeParts {
    let total = (serial * 86_400.0).round() as u64;
    DateTimeParts {
        hour: ((total / 3600) % 24) as u8,
        minute: ((total / 60) % 60) as u8,
        second: (total % 60) as u8,
        serial,
        kind: DateTimeKind::Duration,
        ..date(1900, 1, 1)
    }
}

fn fmt_dt(code: &str, parts: DateTimeParts) -> String {
    NumberFormat::parse(code).format_datetime(&parts).text
}

#[test]
fn the_acceptance_quartet_from_the_probe_workbook() {
    assert_eq!(
        fmt_dt("yyyy\"年\"m\"月\"d\"日\"(aaa)", date(2026, 8, 31)),
        "2026年8月31日(月)"
    );
    assert_eq!(
        fmt_dt("[$-411]ggge\"年\"m\"月\"d\"日\"", date(2026, 8, 31)),
        "令和8年8月31日"
    );
    assert_eq!(fmt_dt("h:mm", time(13, 5, 0)), "13:05");
    assert_eq!(fmt_dt("[h]:mm", duration(1.5)), "36:00");
}

#[test]
fn year_month_day_padding_variants() {
    assert_eq!(fmt_dt("yyyy/m/d", date(2026, 8, 5)), "2026/8/5");
    assert_eq!(fmt_dt("yy/mm/dd", date(2026, 8, 5)), "26/08/05");
}

#[test]
fn english_month_and_weekday_names() {
    assert_eq!(
        fmt_dt("ddd, mmm d, yyyy", date(2026, 8, 31)),
        "Mon, Aug 31, 2026"
    );
    assert_eq!(fmt_dt("dddd", date(2026, 8, 31)), "Monday");
    assert_eq!(fmt_dt("mmmm", date(2026, 8, 31)), "August");
}

#[test]
fn japanese_weekdays() {
    assert_eq!(
        fmt_dt("m\"月\"d\"日\"(aaaa)", date(2026, 8, 31)),
        "8月31日(月曜日)"
    );
    assert_eq!(fmt_dt("aaa", date(2026, 9, 6)), "日");
}

#[test]
fn japanese_era_variants() {
    assert_eq!(fmt_dt("ge.m.d", date(2026, 8, 31)), "R8.8.31");
    assert_eq!(fmt_dt("gg e\"年\"", date(2019, 5, 1)), "令 1年");
    assert_eq!(
        fmt_dt("ggge\"年\"m\"月\"d\"日\"", date(1989, 1, 7)),
        "昭和64年1月7日"
    );
    assert_eq!(fmt_dt("ggge\"年\"", date(2019, 4, 30)), "平成31年");
    assert_eq!(fmt_dt("ee", date(2019, 5, 1)), "01");
}

#[test]
fn am_pm_switches_hours_to_the_twelve_hour_clock() {
    assert_eq!(fmt_dt("h:mm AM/PM", time(13, 5, 0)), "1:05 PM");
    assert_eq!(fmt_dt("h:mm AM/PM", time(0, 30, 0)), "12:30 AM");
    assert_eq!(fmt_dt("h:mm AM/PM", time(12, 0, 0)), "12:00 PM");
    assert_eq!(fmt_dt("h:mm", time(13, 5, 0)), "13:05", "no AM/PM, 24-hour");
    assert!(
        NumberFormat::parse("h:mm A/P").is_general(),
        "the A/P half-token is out of the subset and must degrade"
    );
}

#[test]
fn elapsed_time_counts_past_twenty_four_hours() {
    assert_eq!(
        fmt_dt("[h]:mm:ss", duration(1.5107638888888888)),
        "36:15:30"
    );
    assert_eq!(fmt_dt("[hh]:mm", duration(0.0625)), "01:30");
    assert_eq!(fmt_dt("[m]", duration(2.05)), "2952");
    assert_eq!(fmt_dt("[s]", duration(0.5)), "43200");
    // f64 dust must not shave a minute off (13:05 is 0.54513‥)
    assert_eq!(fmt_dt("[m]", time(13, 5, 0)), "785");
}

#[test]
fn a_lone_m_reads_as_month_next_to_dates_and_minute_next_to_time() {
    assert_eq!(fmt_dt("mm:ss", time(13, 5, 7)), "05:07");
    assert_eq!(fmt_dt("h\"時\"mm\"分\"", time(9, 3, 0)), "9時03分");
    assert_eq!(
        fmt_dt("h\"時\"mm\"分\"ss\"秒\"", time(13, 5, 7)),
        "13時05分07秒"
    );
    assert_eq!(fmt_dt("yyyy/mm", date(2026, 8, 31)), "2026/08");
}

#[test]
fn a_date_format_on_a_bare_number_shows_the_raw_value() {
    let format = NumberFormat::parse("yyyy/m/d");
    assert!(!format.is_general());
    assert!(format.is_date());
    assert_eq!(format.format(46_255.0).text, "46255");
}

#[test]
fn a_numeric_format_on_a_date_cell_paints_the_serial() {
    let numeric = NumberFormat::parse("0.00");
    assert!(!numeric.is_date());
    assert_eq!(numeric.format_datetime(&duration(1.5)).text, "1.50");
    let general = NumberFormat::parse("General");
    assert!(!general.is_date());
    assert_eq!(general.format_datetime(&duration(1.5)).text, "1.5");
}

#[test]
fn date_formats_carry_colors_too() {
    let colored = NumberFormat::parse("[赤]yyyy/m/d");
    assert_eq!(
        colored.format_datetime(&date(2026, 8, 31)).color,
        Some(NamedColor::Red)
    );
}

#[test]
fn a_trailing_text_section_is_ignored() {
    assert_eq!(fmt_dt("yyyy/m/d;@", date(2026, 8, 31)), "2026/8/31");
    assert_eq!(fmt("#,##0;@", 1234.0), "1,234");
    assert!(NumberFormat::parse("@").is_general(), "text-only stays out");
}
