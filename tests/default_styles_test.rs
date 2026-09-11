use std::path::PathBuf;

use docrev::adapter::xlsx_source::XlsxSource;
use docrev::app::ports::DocumentSource;
use docrev::domain::sheet::{Rgb, TextColor};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

const YELLOW: Rgb = Rgb {
    r: 255,
    g: 255,
    b: 0,
};
const GREEN: Rgb = Rgb { r: 0, g: 255, b: 0 };

#[test]
fn empty_cells_take_their_column_and_row_fills() {
    let document = XlsxSource
        .load(&fixture("default_styles.xlsx"))
        .unwrap()
        .into_workbook()
        .unwrap();
    let sheet = &document.sheets()[0];
    assert_eq!(
        sheet.fill_at(0, 0),
        None,
        "A1 has a value but no style of its own, so Excel shows it unfilled"
    );
    assert_eq!(sheet.fill_at(3, 0), Some(YELLOW), "A4 has no <c> element");
    assert_eq!(sheet.fill_at(3, 1), None, "B4 has a value and no style");
    assert_eq!(sheet.fill_at(2, 0), Some(GREEN), "row 3 beats column A");
    assert_eq!(sheet.fill_at(2, 1), Some(GREEN), "B3 has no <c> element");
    assert_eq!(
        sheet.fill_at(2, 2),
        None,
        "C3 is blank but written with a bold-only style of its own"
    );
    assert_eq!(sheet.fill_at(4, 0), None, "A5 likewise, inside the column");
    assert_eq!(sheet.fill_at(1, 1), None);
}

#[test]
fn indexed_and_auto_colors_arrive() {
    let document = XlsxSource
        .load(&fixture("default_styles.xlsx"))
        .unwrap()
        .into_workbook()
        .unwrap();
    let sheet = &document.sheets()[0];
    assert_eq!(sheet.fill_at(0, 2), Some(YELLOW), "indexed 13");
    assert_eq!(
        sheet.text_color_at(0, 2),
        Some(TextColor::Literal(Rgb { r: 255, g: 0, b: 0 })),
        "indexed 10"
    );
    assert_eq!(sheet.text_color_at(1, 2), None, "auto is no color");
}
