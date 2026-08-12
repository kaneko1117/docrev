use std::path::PathBuf;

use docrev::adapter::xlsx_source::XlsxSource;
use docrev::app::ports::DocumentSource;
use docrev::infra::xlsx_meta;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn reads_custom_column_widths_from_the_workbook() {
    let widths = xlsx_meta::column_widths(&fixture("widths.xlsx")).unwrap();
    let cols = widths.get("広い").expect("sheet with custom widths");
    assert!(
        cols.iter()
            .any(|c| c.min == 1 && c.max == 1 && c.width.round() == 20.0),
        "column A should be width 20: {cols:?}"
    );
    assert!(
        cols.iter()
            .any(|c| c.min == 2 && c.max == 2 && c.width.round() == 6.0),
        "column B should be width 6: {cols:?}"
    );
}

#[test]
fn loaded_sheets_carry_their_column_widths() {
    let document = XlsxSource.load(&fixture("widths.xlsx")).unwrap();
    let wide = &document.sheets()[0];
    assert_eq!(wide.name(), "広い");
    assert_eq!(wide.col_width(0), Some(20));
    assert_eq!(wide.col_width(1), Some(6));

    let plain = &document.sheets()[1];
    assert_eq!(plain.name(), "標準");
    assert_eq!(plain.col_width(0), None, "no custom width set");
}

#[test]
fn width_parsing_failure_does_not_block_opening() {
    // basic.xlsx has no custom widths; loading must simply yield defaults
    let document = XlsxSource.load(&fixture("basic.xlsx")).unwrap();
    assert_eq!(document.sheets()[0].col_width(0), None);
}
