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
    assert_eq!(fmt("[$¥-411]#,##0", 1200.0), "¥1,200");
}

#[test]
fn negative_and_zero_sections() {
    let code = "#,##0;▲#,##0;\"-\"";
    assert_eq!(fmt(code, 1234.0), "1,234");
    assert_eq!(fmt(code, -1234.0), "▲1,234");
    assert_eq!(fmt(code, 0.0), "-");
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
        "mm:ss.00",      // fractional seconds mix digits into a date
        "0.0\"日\"yyyy", // digits and date parts in one section
        "[DBNum1]yyyy",  // kanji numerals
        "0.0 ?/?",       // decimals next to a fraction
        "E+00",          // an exponent with no mantissa
        "@0",            // text composed with digits
    ] {
        let format = NumberFormat::parse(code);
        assert!(format.is_general(), "{code} should fall back");
        assert_eq!(format.format(1.5).text, "1.5");
    }
}

#[test]
fn an_unsupported_section_degrades_alone() {
    let code = "[DBNum1]0;\"neg\"0";
    assert_eq!(
        fmt(code, 5.0),
        "5",
        "the raw value stands in for the first section"
    );
    assert_eq!(fmt(code, -5.0), "neg5", "the second section still works");
    assert!(!NumberFormat::parse(code).is_general());
}

#[test]
fn conditional_sections_pick_by_comparison_and_keep_the_sign() {
    let code = "[>=1000]#,##0,\"千\";0";
    assert_eq!(fmt(code, 1500.0), "2千");
    assert_eq!(fmt(code, 5.0), "5");
    assert_eq!(fmt(code, -5.0), "-5", "no sign dropping with conditions");
    assert_eq!(
        fmt("[>=1000]#,##0\"千\";0", 1500.0),
        "1,500千",
        "no scaling comma"
    );
    let two = "[<0]\"neg \"0;[>=100]\"big \"0;\"mid \"0";
    assert_eq!(
        fmt(two, -5.0),
        "-neg 5",
        "the sign leads, as with currency literals"
    );
    assert_eq!(fmt(two, 250.0), "big 250");
    assert_eq!(fmt(two, 50.0), "mid 50");
    assert_eq!(fmt("[<>0]0;\"zero\"", 0.0), "zero");
    assert_eq!(fmt("[=5]\"five\";0", 5.0), "five");
    assert_eq!(
        fmt("[<=1]0", 7.0),
        "7",
        "no otherwise section: the raw value"
    );
    let colored = NumberFormat::parse("[Red][<0]0;0");
    assert_eq!(colored.format(-3.0).color, Some(NamedColor::Red));
    assert_eq!(colored.format(3.0).color, None);
    assert_eq!(
        fmt("[DBNum1][>=1000]0;0", -5.0),
        "-5",
        "a degraded conditional section still keeps the code conditional"
    );
    assert_eq!(
        fmt("[DBNum1][>=1000]0;0", 1500.0),
        "1500",
        "raw value for the degraded one"
    );
}

#[test]
fn scientific_notation() {
    assert_eq!(fmt("0.00E+00", 12345.0), "1.23E+04");
    assert_eq!(fmt("0.00E+00", -12345.0), "-1.23E+04");
    assert_eq!(fmt("0.00E+00", 0.00012), "1.20E-04");
    assert_eq!(fmt("0.00E+00", 0.0), "0.00E+00");
    assert_eq!(
        fmt("0.00E-00", 12345.0),
        "1.23E04",
        "E- writes only a negative sign"
    );
    assert_eq!(fmt("0.00E-00", 0.5), "5.00E-01");
    assert_eq!(
        fmt("##0.0E+0", 12345.0),
        "12.3E+3",
        "engineering steps of three"
    );
    assert_eq!(fmt("##0.0E+0", 0.0123), "12.3E-3");
    assert_eq!(
        fmt("0.0E+0", 9.99),
        "1.0E+1",
        "rounding carries into the exponent"
    );
    assert_eq!(fmt("0E+0", 7.0), "7E+0");
    assert_eq!(
        fmt("0.00e+00", 12345.0),
        "1.23e+04",
        "the letter keeps its case"
    );
    assert_eq!(
        fmt("#,##0.0E+0", 1234.5),
        "1,234.5E+0",
        "grouping commas are not digits"
    );
}

#[test]
fn fractions() {
    assert_eq!(fmt("# ?/?", 0.5), "1/2");
    assert_eq!(fmt("# ?/?", 1.5), "1 1/2");
    assert_eq!(fmt("# ?/?", 3.0), "3", "a whole number drops the fraction");
    assert_eq!(fmt("# ?/?", 0.0), "0");
    assert_eq!(fmt("# ?/?", 0.333), "1/3");
    assert_eq!(fmt("# ??/??", 0.3125), "5/16");
    assert_eq!(
        fmt("# ?/?", 0.3125),
        "1/3",
        "one digit: the closest of 1..9"
    );
    assert_eq!(
        fmt("# ?/8", 0.3),
        "2/8",
        "a fixed denominator is not reduced"
    );
    assert_eq!(fmt("# ?/8", 2.0), "2");
    assert_eq!(fmt("?/?", 1.5), "3/2", "no whole placeholder: improper");
    assert_eq!(fmt("?/?", 2.0), "2/1");
    assert_eq!(fmt("?/?", 0.0), "0/1");
    assert_eq!(fmt("# ?/?", -1.25), "-1 1/4");
    assert_eq!(fmt("0 0/0", 0.999), "1", "rounding up folds into the whole");
    assert_eq!(
        fmt("0 ?/?", 0.5),
        "0 1/2",
        "a forced whole digit shows the zero"
    );
    assert_eq!(
        fmt("#,##0 ?/?", 1234.5),
        "1,234 1/2",
        "the whole keeps its grouping"
    );
    assert_eq!(fmt("00 ?/?", 1.5), "01 1/2");
    assert_eq!(
        fmt("?/8", 2.0),
        "16/8",
        "a fixed denominator stays improper"
    );
    assert_eq!(fmt("# ?/?", 1e20), "100000000000000000000");
    assert_eq!(fmt("?/?", 1e20), "100000000000000000000/1");
    assert_eq!(
        fmt("#,##0.00", 0.5),
        "0.50",
        "a decimal point is not a fraction"
    );
}

#[test]
fn text_sections_compose_text_and_turn_numbers_into_text() {
    let plain = NumberFormat::parse("@");
    assert!(!plain.is_general());
    assert!(plain.is_text_only());
    assert_eq!(plain.format_text("abc").map(|f| f.text), Some("abc".into()));
    assert_eq!(plain.format(42.0).text, "42");
    let styled = NumberFormat::parse("[Blue]@\"様\"");
    let formatted = styled.format_text("田中").unwrap();
    assert_eq!(formatted.text, "田中様");
    assert_eq!(formatted.color, Some(NamedColor::Blue));
    let mixed = NumberFormat::parse("#,##0;@");
    assert!(!mixed.is_text_only());
    assert_eq!(mixed.format_text("x").map(|f| f.text), Some("x".into()));
    assert_eq!(
        mixed.format(-1234.0).text,
        "-1,234",
        "one numeric section keeps the sign"
    );
    assert_eq!(NumberFormat::parse("#,##0").format_text("x"), None);
    assert_eq!(
        NumberFormat::parse("0;0;0;@")
            .format_text("x")
            .map(|f| f.text),
        Some("x".into()),
        "the fourth section is the text one"
    );
}

#[test]
fn general_matches_the_raw_rendering() {
    let format = NumberFormat::parse("General");
    assert_eq!(format.format(120.0).text, "120");
    assert_eq!(format.format(80.5).text, "80.5");
}

#[test]
fn absurdly_long_codes_degrade_to_general_instantly() {
    let bomb = "0".repeat(5_000_000);
    let format = NumberFormat::parse(&bomb);
    assert!(format.is_general());
    assert_eq!(format.format(1.5).text, "1.5");

    let frac_bomb = format!("0.{}", "0".repeat(5_000_000));
    assert!(NumberFormat::parse(&frac_bomb).is_general());
}

#[test]
fn the_longest_accepted_code_still_formats_in_bounded_time() {
    let widest = "0".repeat(512);
    let format = NumberFormat::parse(&widest);
    assert!(!format.is_general(), "512 chars is within the cap");
    let text = format.format(7.0).text;
    assert_eq!(text.len(), 512);
    assert!(text.ends_with('7'));
}

#[test]
fn decimal_places_clamp_at_excels_thirty() {
    // 10^510 overflows f64
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
    NumberFormat::parse(code)
        .format_datetime(&parts)
        .map(|f| f.text)
        .unwrap_or_else(|| "<unsupported>".into())
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
    // 13:05 is 0.54513‥
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
    assert_eq!(fmt_dt("0.00", duration(1.5)), "1.50");
    let general = NumberFormat::parse("General");
    assert!(!general.is_date());
    assert_eq!(fmt_dt("General", duration(1.5)), "1.5");
    assert_eq!(
        fmt_dt("mm:ss.0;@", duration(1.5)),
        "<unsupported>",
        "a degraded first section hands the date back to the caller"
    );
}

#[test]
fn date_formats_carry_colors_too() {
    let colored = NumberFormat::parse("[赤]yyyy/m/d");
    assert_eq!(
        colored.format_datetime(&date(2026, 8, 31)).map(|f| f.color),
        Some(Some(NamedColor::Red))
    );
}

#[test]
fn the_word_general_in_a_section_renders_the_raw_value() {
    let format = NumberFormat::parse("General;[Red]-General");
    assert!(!format.is_general(), "the sections are understood");
    assert!(
        !format.is_date(),
        "General's G and e must never read as era tokens"
    );
    assert_eq!(format.format(46_265.0).text, "46265");
    assert_eq!(format.format(-46_265.0).text, "-46265");
    assert_eq!(format.format(-46_265.0).color, Some(NamedColor::Red));
    assert_eq!(fmt("General;@", 1.5), "1.5");
    assert!(NumberFormat::parse("General yyyy").is_general());
    assert!(NumberFormat::parse("0 General").is_general());
}

#[test]
fn negative_durations_keep_their_minutes_and_seconds() {
    // negative durations: the calendar parts saturate to 0
    assert_eq!(
        fmt_dt("[h]:mm:ss", duration(-108_900.0 / 86_400.0)),
        "-30:15:00"
    );
    assert_eq!(fmt_dt("[h]:mm", duration(108_900.0 / 86_400.0)), "30:15");
}

#[test]
fn a_trailing_text_section_does_not_touch_numbers_or_dates() {
    assert_eq!(fmt_dt("yyyy/m/d;@", date(2026, 8, 31)), "2026/8/31");
    assert_eq!(fmt("#,##0;@", 1234.0), "1,234");
    assert!(NumberFormat::parse("yyyy/m/d;@").is_date());
}
