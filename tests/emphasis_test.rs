use std::path::PathBuf;

use docrev::adapter::xlsx_source::XlsxSource;
use docrev::app::dump::dump;
use docrev::app::ports::DocumentSource;
use docrev::domain::sheet::{Emphasis, Rgb, TextColor};
use docrev::ui::table::render;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn loaded_sheets_carry_font_emphasis_but_not_underline() {
    let document = XlsxSource.load(&fixture("emphasis.xlsx")).unwrap();
    let sheet = &document.sheets()[0];
    let bold = Emphasis {
        bold: true,
        ..Emphasis::default()
    };
    let strike = Emphasis {
        strike: true,
        ..Emphasis::default()
    };
    let italic = Emphasis {
        italic: true,
        ..Emphasis::default()
    };
    assert_eq!(sheet.display_emphasis_at(0, 0), bold);
    assert_eq!(sheet.display_emphasis_at(1, 0), strike);
    assert_eq!(sheet.display_emphasis_at(2, 0), italic);
    assert_eq!(
        sheet.display_emphasis_at(3, 0),
        Emphasis::default(),
        "underline is not carried"
    );
    assert_eq!(sheet.display_emphasis_at(4, 0), Emphasis::default());
    assert_eq!(
        sheet.display_emphasis_at(5, 0),
        Emphasis {
            bold: true,
            italic: true,
            strike: true,
        }
    );
    assert_eq!(
        sheet.text_color_at(5, 0),
        Some(TextColor::Literal(Rgb { r: 255, g: 0, b: 0 })),
        "the color of the same font still arrives"
    );
}

#[test]
fn the_dump_stays_plain_text() {
    let view = dump(&XlsxSource, &fixture("emphasis.xlsx"), None).unwrap();
    let out = render(&view.sheet, view.position, view.total, false);
    assert!(out.contains("│ 1 │ 見出し   │"), "{out}");
    assert!(out.contains("│ 2 │ 削除済み │"), "{out}");
    assert!(!out.contains('\u{1b}'), "no escape sequences in the dump");
}
