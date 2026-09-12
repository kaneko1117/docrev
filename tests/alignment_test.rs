use std::path::PathBuf;

use docrev::adapter::xlsx_source::XlsxSource;
use docrev::app::dump::dump;
use docrev::app::ports::DocumentSource;
use docrev::domain::sheet::{Alignment, Horizontal, Vertical};
use docrev::ui::table::render;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn loaded_sheets_carry_cell_alignment() {
    let document = XlsxSource
        .load(&fixture("alignment.xlsx"))
        .unwrap()
        .into_workbook()
        .unwrap();
    let sheet = &document.sheets()[0];
    assert_eq!(
        sheet.alignment_at(0, 0).and_then(|a| a.horizontal),
        Some(Horizontal::Center)
    );
    assert_eq!(
        sheet.alignment_at(2, 0).and_then(|a| a.horizontal),
        Some(Horizontal::Right)
    );
    assert_eq!(
        sheet.alignment_at(3, 0),
        Some(Alignment {
            horizontal: Some(Horizontal::Left),
            vertical: Some(Vertical::Bottom),
            indent: 1,
        })
    );
    assert_eq!(sheet.alignment_at(1, 0), None, "general is nothing");
    assert_eq!(sheet.alignment_at(3, 1), None, "wrapText alone is nothing");
    assert_eq!(
        sheet.alignment_at(5, 1),
        None,
        "the merged-away cell has none of its own"
    );
    assert_eq!(
        sheet.display_alignment_at(5, 1).and_then(|a| a.horizontal),
        Some(Horizontal::Center),
        "inside the merge, the anchor's alignment"
    );
}

#[test]
fn the_dump_centers_headers_and_indents_labels() {
    let view = dump(&XlsxSource, &fixture("alignment.xlsx"), None, false).unwrap();
    let (position, total) = view.place().unwrap();
    let out = render(view.sheet().unwrap(), position, total, false);
    assert!(out.contains("│ 1 │  商品  │"), "centered header:\n{out}");
    assert!(
        out.contains("│ 3 │   合計 │"),
        "right-aligned label:\n{out}"
    );
    assert!(
        out.contains("│ 4 │        │ 説明") && out.contains("│   │   内訳 │ です"),
        "indented label on the bottom line of its wrapped row:\n{out}"
    );
}
