use std::path::PathBuf;

use docrev::adapter::xlsx_source::XlsxSource;
use docrev::app::ports::DocumentSource;
use docrev::domain::sheet::{Rgb, TextColor};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

const RED: Rgb = Rgb { r: 255, g: 0, b: 0 };
const YELLOW: Rgb = Rgb {
    r: 255,
    g: 255,
    b: 0,
};
const GREEN: Rgb = Rgb { r: 0, g: 255, b: 0 };

#[test]
fn flagged_cells_take_their_rules_formats() {
    let document = XlsxSource.load(&fixture("conditional.xlsx")).unwrap();
    let sheet = &document.sheets()[0];
    // B: > 100 red (priority 1) beats > 400 yellow; bold when B beats the same row's C
    assert_eq!(sheet.fill_at(1, 1), None, "50");
    assert_eq!(
        sheet.fill_at(2, 1),
        Some(RED),
        "500 breaches both, red wins"
    );
    assert_eq!(sheet.fill_at(3, 1), None, "80");
    assert_eq!(sheet.fill_at(4, 1), Some(RED), "120");
    assert!(sheet.display_emphasis_at(2, 1).bold, "500 > 100");
    assert!(sheet.display_emphasis_at(3, 1).bold, "80 > 60");
    assert!(!sheet.display_emphasis_at(4, 1).bold, "120 < 200");
    // D: contains "ng" red font, case-insensitive; blanks green
    assert_eq!(
        sheet.text_color_at(2, 3),
        Some(TextColor::Literal(RED)),
        "NG"
    );
    assert_eq!(sheet.text_color_at(1, 3), None, "OK");
    assert_eq!(sheet.fill_at(4, 3), Some(GREEN), "blank");
    // A: duplicates yellow
    assert_eq!(sheet.fill_at(1, 0), Some(YELLOW));
    assert_eq!(sheet.fill_at(4, 0), Some(YELLOW));
    assert_eq!(sheet.fill_at(2, 0), None);
    // C: the expression rule is skipped, so nothing in C is colored
    for row in 1..=4 {
        assert_eq!(sheet.fill_at(row, 2), None, "C{}", row + 1);
    }
}

#[test]
fn workbooks_without_rules_are_unchanged() {
    let document = XlsxSource.load(&fixture("fills.xlsx")).unwrap();
    assert!(!document.sheets().is_empty());
}
