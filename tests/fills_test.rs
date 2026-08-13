use std::path::PathBuf;

use docrev::adapter::xlsx_source::XlsxSource;
use docrev::app::ports::DocumentSource;
use docrev::domain::cell::CellValue;
use docrev::domain::sheet::Rgb;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn loaded_sheets_carry_their_fill_colors() {
    let document = XlsxSource.load(&fixture("fills.xlsx")).unwrap();
    let sheet = &document.sheets()[0];
    assert_eq!(sheet.name(), "塗り");
    assert_eq!(sheet.fill_at(0, 0), Some(Rgb { r: 255, g: 0, b: 0 }));
    assert_eq!(
        sheet.fill_at(0, 1),
        Some(Rgb {
            r: 0x00,
            g: 0xB0,
            b: 0x50
        }),
        "fills apply to cells without a value"
    );
    assert_eq!(sheet.fill_at(0, 2), None, "unfilled cells stay unpainted");
}

#[test]
fn fills_and_number_formats_coexist_on_one_cell() {
    let document = XlsxSource.load(&fixture("fills.xlsx")).unwrap();
    let sheet = &document.sheets()[0];
    assert_eq!(
        sheet.cell(0, 3),
        &CellValue::FormattedNumber {
            value: 0.15,
            text: "15%".to_string(),
            color: None,
        }
    );
    assert_eq!(
        sheet.fill_at(0, 3),
        Some(Rgb {
            r: 255,
            g: 255,
            b: 0
        })
    );
}

#[test]
fn a_broken_theme_drops_theme_fills_but_keeps_rgb_fills() {
    // fills_broken_theme.xlsx is fills.xlsx with an incomplete clrScheme:
    // guessing colors would silently diverge from Excel, so theme-indexed
    // fills disappear while explicit rgb fills stay
    let document = XlsxSource
        .load(&fixture("fills_broken_theme.xlsx"))
        .unwrap();
    let sheet = &document.sheets()[0];
    assert_eq!(sheet.fill_at(0, 0), Some(Rgb { r: 255, g: 0, b: 0 }));
    assert_eq!(sheet.fill_at(0, 4), None, "theme fill must not be guessed");
}

#[test]
fn theme_fills_resolve_through_the_workbook_theme() {
    // openpyxl writes the 2007 theme (accent1 #4F81BD), not the current
    // Office default (#4472C4) — only reading theme1.xml produces this
    // value, so a hardcoded-palette regression fails here. tint 0.4
    // lightens each channel 40% toward white.
    let document = XlsxSource.load(&fixture("fills.xlsx")).unwrap();
    let sheet = &document.sheets()[0];
    assert_eq!(
        sheet.fill_at(0, 4),
        Some(Rgb {
            r: 149,
            g: 179,
            b: 215
        })
    );
}
