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
fn hidden_columns_and_rows_are_read_from_the_workbook() {
    let meta = xlsx_meta::read_meta(&fixture("hidden.xlsx"));
    let cols = meta.cols.get("表").expect("sheet with a hidden column");
    assert!(
        cols.iter().any(|c| c.min == 3 && c.max == 3 && c.hidden),
        "column C is hidden: {cols:?}"
    );
    assert!(
        cols.iter()
            .any(|c| c.min == 4 && c.max == 4 && !c.hidden && c.width.is_some()),
        "column D keeps its width and is visible: {cols:?}"
    );
    assert_eq!(meta.hidden_rows.get("表"), Some(&vec![2, 4]));
}

#[test]
fn loaded_sheets_carry_hidden_rows_columns_and_sheet_state() {
    let document = XlsxSource.load(&fixture("hidden.xlsx")).unwrap();
    let names: Vec<&str> = document.sheet_names().collect();
    assert_eq!(
        names,
        vec!["表", "作業用", "secret"],
        "hidden sheets are loaded"
    );

    let table = &document.sheets()[0];
    assert!(!table.is_hidden());
    assert!(table.col_hidden(2), "C");
    assert!(!table.col_hidden(3), "D");
    assert!(table.row_hidden(2) && table.row_hidden(4), "rows 3 and 5");
    assert!(!table.row_hidden(0) && !table.row_hidden(3));
    assert_eq!(table.col_width(3).map(f64::round), Some(14.0));

    assert!(document.sheets()[1].is_hidden(), "hidden");
    assert!(document.sheets()[2].is_hidden(), "veryHidden");
}

#[test]
fn a_workbook_without_hidden_parts_reports_none() {
    let document = XlsxSource.load(&fixture("basic.xlsx")).unwrap();
    let sheet = &document.sheets()[0];
    assert!(!sheet.is_hidden());
    assert!((0..sheet.row_count()).all(|r| !sheet.row_hidden(r)));
    assert!((0..sheet.col_count()).all(|c| !sheet.col_hidden(c)));
}
