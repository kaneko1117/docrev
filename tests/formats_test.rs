use std::path::PathBuf;

use docrev::adapter::xlsx_source::XlsxSource;
use docrev::app::ports::DocumentSource;
use docrev::domain::cell::CellValue;
use docrev::domain::number_format::FormatColor;
use docrev::infra::xlsx_meta;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn reads_format_codes_from_the_workbook() {
    let styles = xlsx_meta::read_meta(&fixture("formats.xlsx")).styles;
    let sheet = styles.sheets.get("書式").expect("sheet with styles");
    let code_of = |pos: (u32, u32)| {
        sheet
            .get(&pos)
            .and_then(|&i| styles.styles[i].format.as_deref())
    };
    // built-in ids resolve without a numFmt entry in styles.xml
    assert_eq!(code_of((0, 0)), Some("0%"));
    assert_eq!(code_of((0, 1)), Some("#,##0"));
    // custom ids resolve through numFmts
    assert_eq!(code_of((0, 2)), Some("#,##0;[Red]▲#,##0"));
    assert_eq!(code_of((0, 5)), None, "unformatted cells carry no code");
}

#[test]
fn loaded_cells_display_as_excel_shows_them() {
    let document = XlsxSource.load(&fixture("formats.xlsx")).unwrap();
    let sheet = &document.sheets()[0];
    assert_eq!(sheet.name(), "書式");

    let formatted =
        |text: &str, value: f64, color: Option<FormatColor>| CellValue::FormattedNumber {
            value,
            text: text.to_string(),
            color,
        };
    assert_eq!(sheet.cell(0, 0), &formatted("15%", 0.15, None));
    assert_eq!(sheet.cell(0, 1), &formatted("1,234", 1234.0, None));
    assert_eq!(
        sheet.cell(0, 2),
        &formatted("▲1,234", -1234.0, Some(FormatColor::Red))
    );
    assert_eq!(sheet.cell(0, 3), &formatted("1,235千円", 1234567.0, None));
    assert_eq!(sheet.cell(0, 4), &formatted("¥1,980", 1980.0, None));
}

#[test]
fn unformatted_numbers_and_dates_keep_their_existing_behavior() {
    let document = XlsxSource.load(&fixture("formats.xlsx")).unwrap();
    let sheet = &document.sheets()[0];
    assert_eq!(sheet.cell(0, 5), &CellValue::Number(42.0));
    assert!(
        matches!(sheet.cell(1, 0), CellValue::DateTime(_)),
        "date cells stay on the calamine conversion: {:?}",
        sheet.cell(1, 0)
    );
}

#[test]
fn format_parsing_failure_does_not_block_opening() {
    // corrupt_styles.xlsx is formats.xlsx with xl/styles.xml removed:
    // resolving formats fails, opening must degrade to raw values
    let path = fixture("corrupt_styles.xlsx");
    let mut archive = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
    assert!(
        archive.by_name("xl/styles.xml").is_err(),
        "the fixture must actually exercise the failure path"
    );

    let document = XlsxSource.load(&path).unwrap();
    let sheet = &document.sheets()[0];
    assert_eq!(sheet.cell(0, 0), &CellValue::Number(0.15));
    assert_eq!(sheet.cell(0, 2), &CellValue::Number(-1234.0));
}

#[test]
fn formatless_workbooks_stay_raw() {
    let document = XlsxSource.load(&fixture("basic.xlsx")).unwrap();
    let sheet = &document.sheets()[0];
    for row in 0..sheet.row_count() {
        for col in 0..sheet.col_count() {
            assert!(
                !matches!(sheet.cell(row, col), CellValue::FormattedNumber { .. }),
                "unexpected formatted cell at ({row}, {col})"
            );
        }
    }
}
