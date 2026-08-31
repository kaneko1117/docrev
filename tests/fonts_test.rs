use std::path::PathBuf;

use docrev::adapter::xlsx_source::XlsxSource;
use docrev::app::ports::DocumentSource;
use docrev::domain::sheet::{Rgb, TextColor};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn loaded_sheets_carry_their_font_colors() {
    let document = XlsxSource.load(&fixture("fonts.xlsx")).unwrap();
    let sheet = &document.sheets()[0];
    assert_eq!(sheet.name(), "フォント");
    assert_eq!(
        sheet.text_color_at(0, 0),
        Some(TextColor::Literal(Rgb {
            r: 255,
            g: 255,
            b: 255
        })),
        "white font on the dark fill"
    );
    assert_eq!(
        sheet.text_color_at(0, 1),
        Some(TextColor::Literal(Rgb { r: 255, g: 0, b: 0 }))
    );
    assert_eq!(
        sheet.text_color_at(0, 3),
        None,
        "the default black font is not inherited"
    );
}

#[test]
fn font_colors_and_dark_fills_arrive_together() {
    // the cell that motivated #32: readable in Excel because the author
    // paired a dark fill with a white font — both must survive the trip
    let document = XlsxSource.load(&fixture("fonts.xlsx")).unwrap();
    let sheet = &document.sheets()[0];
    assert_eq!(
        sheet.fill_at(0, 0),
        Some(Rgb {
            r: 0x20,
            g: 0x38,
            b: 0x64
        })
    );
    assert_eq!(
        sheet.text_color_at(0, 0),
        Some(TextColor::Literal(Rgb {
            r: 255,
            g: 255,
            b: 255
        }))
    );
}
