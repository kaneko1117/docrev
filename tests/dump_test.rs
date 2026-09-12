use std::path::PathBuf;

use docrev::adapter::xlsx_source::XlsxSource;
use docrev::app::dump::dump;
use docrev::ui::table::render;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn dumps_first_sheet_by_default() {
    let view = dump(&XlsxSource, &fixture("basic.xlsx"), None, false).unwrap();
    let (position, total) = view.place().unwrap();
    let out = render(view.sheet().unwrap(), position, total, false);
    assert!(out.contains("Sheet: 売上 (1/1)"));
    assert!(out.contains("│ りんご"));
    assert!(out.contains(" 120 │"), "numbers should be right-aligned");
}

#[test]
fn selects_sheet_by_name() {
    let view = dump(
        &XlsxSource,
        &fixture("multi_sheet.xlsx"),
        Some("集計"),
        false,
    )
    .unwrap();
    assert_eq!(view.sheet().unwrap().name(), "集計");
    assert_eq!(view.place(), Some((2, 3)));
}

#[test]
fn unknown_sheet_lists_available_names() {
    let err = dump(
        &XlsxSource,
        &fixture("multi_sheet.xlsx"),
        Some("ない"),
        false,
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("\"ない\" not found"), "unexpected: {msg}");
    for name in ["売上", "経費", "集計"] {
        assert!(msg.contains(name), "missing {name} in: {msg}");
    }
}

#[test]
fn renders_edge_cases() {
    let view = dump(&XlsxSource, &fixture("edge.xlsx"), None, false).unwrap();
    let (position, total) = view.place().unwrap();
    let out = render(view.sheet().unwrap(), position, total, false);
    assert!(
        !out.contains('…'),
        "long text wraps instead of clipping (#33)"
    );
    assert!(
        out.contains("められるはずの文字列"),
        "the wrapped tail is visible: {out}"
    );
    assert!(out.contains("3.14"));
    assert!(out.contains("42"));
    assert!(out.contains("TRUE"));
    assert!(out.contains("2026-08-09"));
}

#[test]
fn missing_file_is_a_typed_error() {
    let err = dump(&XlsxSource, &fixture("nope.xlsx"), None, false).unwrap_err();
    assert!(err.to_string().contains("nope.xlsx"));
}
