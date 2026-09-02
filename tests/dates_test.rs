use std::path::PathBuf;

use docrev::adapter::xlsx_source::XlsxSource;
use docrev::app::ports::DocumentSource;
use docrev::domain::cell::CellValue;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// The acceptance table of #97: every cell displays as Excel shows it.
#[test]
fn date_cells_display_as_excel_shows_them() {
    let document = XlsxSource.load(&fixture("dates.xlsx")).unwrap();
    let sheet = &document.sheets()[0];
    assert_eq!(sheet.name(), "日付");
    let text = |row: usize| sheet.cell(row, 1).display_text();
    assert_eq!(text(0), "2026年8月31日(月)", "custom weekday");
    assert_eq!(text(1), "令和8年8月31日", "custom era");
    assert_eq!(text(2), "13:05", "time with no date component");
    assert_eq!(text(3), "36:00", "elapsed hours past 24");
    assert_eq!(text(4), "2026/8/31", "standard picker date (builtin id 14)");
    assert_eq!(
        text(5),
        "令和8年8月31日",
        "standard picker era (builtin id 28)"
    );
    assert_eq!(
        text(6),
        "R8.8.31",
        "standard picker short era (builtin id 27)"
    );
    assert_eq!(text(7), "13:05:00", "standard picker time (builtin id 21)");
    assert_eq!(text(8), "2026/8/31 13:05", "date and time combined");
    assert_eq!(text(9), "1:05 PM", "twelve-hour clock");
    assert_eq!(text(10), "Aug 31, Mon", "English month and weekday");
    assert_eq!(text(11), "8月31日(月曜日)", "long Japanese weekday");
    assert_eq!(
        text(12),
        "13:05:00",
        "unsupported format falls back, no epoch"
    );
    assert_eq!(text(13), "2952", "elapsed minutes");
}

/// Agents keep a machine-readable value next to the formatted display.
#[test]
fn date_cells_keep_a_machine_readable_raw() {
    let document = XlsxSource.load(&fixture("dates.xlsx")).unwrap();
    let sheet = &document.sheets()[0];
    let raw = |row: usize| match sheet.cell(row, 1) {
        CellValue::DateTime { raw, .. } => raw.clone(),
        other => panic!("row {row} is not a date cell: {other:?}"),
    };
    assert_eq!(raw(0), "2026-08-31 00:00:00", "date-bearing cell");
    assert_eq!(
        raw(2),
        "13:05:00",
        "time-only cell — no fictional epoch date"
    );
    assert_eq!(raw(3), "36:00:00", "duration counts elapsed time");
    assert_eq!(raw(5), "2026-08-31 00:00:00", "promoted builtin-era cell");
}
