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
fn custom_row_heights_and_sheet_defaults_are_read_from_the_workbook() {
    let meta = xlsx_meta::read_meta(&fixture("heights.xlsx"));
    let rows = meta.rows.get("区切り").expect("sheet with custom heights");
    assert!(
        rows.iter().any(|r| r.index == 2 && r.height == Some(60.0)),
        "row 3 is 60pt: {rows:?}"
    );
    let format = meta.formats.get("広い既定").expect("sheet with defaults");
    assert_eq!(format.default_col_width, Some(20.0));
    assert_eq!(format.default_row_height, Some(18.75));
    assert_eq!(
        meta.formats.get("区切り").and_then(|f| f.default_col_width),
        None,
        "openpyxl's restated base width is not a statement"
    );
}

#[test]
fn loaded_sheets_carry_row_heights_and_default_sizes() {
    let document = XlsxSource.load(&fixture("heights.xlsx")).unwrap();
    let sections = &document.sheets()[0];
    assert_eq!(sections.row_height(2), Some(60.0));
    assert_eq!(sections.row_height(4), Some(20.0));
    assert_eq!(sections.row_height(0), None);
    assert_eq!(sections.default_col_width(), None);

    let wide = &document.sheets()[1];
    assert_eq!(wide.default_col_width(), Some(20.0));
    assert_eq!(wide.default_row_height(), Some(18.75));
    assert_eq!(wide.row_height(0), None);
}
