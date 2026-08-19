use std::path::PathBuf;

use docrev::adapter::xlsx_source::XlsxSource;
use docrev::app::ports::DocumentSource;
use docrev::ui::table::render;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/formulas.xlsx")
}

#[test]
fn formulas_load_and_shared_ones_resolve_per_cell() {
    let document = XlsxSource.load(&fixture()).unwrap();
    let sheet = &document.sheets()[0];
    assert_eq!(sheet.formula_at(0, 1), Some("SUM(A1:A2)"));
    assert_eq!(sheet.formula_at(0, 2), Some("A1*2"), "the shared master");
    assert_eq!(
        sheet.formula_at(1, 2),
        Some("A2*2"),
        "the follower gets its own shifted formula, not a blank"
    );
    assert_eq!(sheet.formula_at(0, 0), None, "a plain value has none");
}

/// Files written without evaluating (openpyxl and friends) carry formulas
/// with no cached result — those cells sit outside the value grid, and the
/// grid must grow to reach them or the formula is silently invisible.
#[test]
fn a_formula_without_a_cached_result_is_still_reachable() {
    let document = XlsxSource.load(&fixture()).unwrap();
    let sheet = &document.sheets()[0];
    assert_eq!(sheet.formula_at(3, 1), Some("A1+A2"));
    assert!(
        sheet.row_count() >= 4,
        "the grid covers the formula-only row"
    );

    let formulas = render(sheet, 0, 1, true);
    assert!(
        formulas.contains("=A1+A2"),
        "the uncached formula must show:\n{formulas}"
    );
}

/// Excel's formula view aligns every formula left, whatever its result type.
#[test]
fn formulas_align_left_even_over_numeric_results() {
    let document = XlsxSource.load(&fixture()).unwrap();
    let sheet = &document.sheets()[0];
    let formulas = render(sheet, 0, 1, true);
    let line = formulas
        .lines()
        .find(|l| l.contains("=SUM"))
        .expect("formula row");
    assert!(
        line.contains("│ =SUM(A1:A2)"),
        "left-aligned right after the border:\n{line}"
    );
}

#[test]
fn dump_shows_results_by_default_and_formulas_on_demand() {
    let document = XlsxSource.load(&fixture()).unwrap();
    let sheet = &document.sheets()[0];

    let plain = render(sheet, 0, 1, false);
    assert!(plain.contains('5'), "the result:\n{plain}");
    assert!(!plain.contains("SUM"), "no formulas uninvited:\n{plain}");

    let formulas = render(sheet, 0, 1, true);
    assert!(formulas.contains("=SUM(A1:A2)"), "{formulas}");
    assert!(formulas.contains("=A2*2"), "shared follower:\n{formulas}");
    assert!(
        formulas.contains('2') && formulas.contains('3'),
        "plain values keep showing:\n{formulas}"
    );
}
