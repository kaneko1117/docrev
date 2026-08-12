use std::path::PathBuf;

use docrev::adapter::xlsx_source::XlsxSource;
use docrev::app::ports::DocumentSource;
use docrev::domain::sheet::MergedRange;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn loaded_sheets_carry_their_merged_regions() {
    let document = XlsxSource.load(&fixture("merged.xlsx")).unwrap();
    let sheet = &document.sheets()[0];

    let horizontal = sheet.merge_at(0, 1).expect("B1 is inside A1:C1");
    assert_eq!(
        horizontal,
        &MergedRange {
            start_row: 0,
            start_col: 0,
            end_row: 0,
            end_col: 2,
        }
    );

    let vertical = sheet.merge_at(2, 0).expect("A3 is inside A2:A3");
    assert_eq!(vertical.anchor(), (1, 0));

    assert!(sheet.merge_at(1, 1).is_none(), "B2 is a normal cell");
}

#[test]
fn documents_without_merges_load_unchanged() {
    let document = XlsxSource.load(&fixture("basic.xlsx")).unwrap();
    assert!(document.sheets()[0].merge_at(0, 0).is_none());
}
