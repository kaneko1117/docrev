use std::path::PathBuf;
use std::process::Command;

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

fn docrev() -> Command {
    Command::new(env!("CARGO_BIN_EXE_docrev"))
}

#[test]
fn dump_shows_what_excel_shows() {
    let out = docrev()
        .args(["dump", fixture("hidden.xlsx").to_str().unwrap()])
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("│ A  │ B  │ D  │ E  │"), "{text}");
    assert!(text.contains("│ 1 │ A1 │ B1 │ D1 │ E1 │"), "{text}");
    assert!(text.contains("│ 4 │ A4 │ B4 │ D4 │ E4 │"), "{text}");
    assert!(
        !text.contains("C1") && !text.contains("A3") && !text.contains("A5"),
        "{text}"
    );
}

#[test]
fn a_thread_on_a_hidden_cell_is_reachable_and_marked() {
    let dir = std::env::temp_dir().join(format!("docrev-hidden-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let document = dir.join("hidden.xlsx");
    std::fs::copy(fixture("hidden.xlsx"), &document).unwrap();
    let path = document.to_str().unwrap();

    let add = docrev()
        .args(["comment", "add", path, "--cell", "表!C3", "--body", "check"])
        .output()
        .unwrap();
    assert!(
        add.status.success(),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );

    let text = docrev().args(["comment", "list", path]).output().unwrap();
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.contains("表!C3 (hidden) [agent] check"), "{text}");

    let json = docrev()
        .args(["comment", "list", "--json", path])
        .output()
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    let entry = &parsed["comments"][0];
    assert_eq!(entry["hidden"], true);
    assert_eq!(
        entry["cell"]["value"], "C3",
        "the anchored cell is still read"
    );
    assert_eq!(
        entry["cell"]["row"],
        serde_json::json!({}),
        "a hidden row shows nothing of its neighbours"
    );

    let visible = docrev()
        .args(["comment", "add", path, "--cell", "表!A2", "--body", "row"])
        .output()
        .unwrap();
    assert!(visible.status.success());
    let json = docrev()
        .args(["comment", "list", "--json", path])
        .output()
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    let entry = &parsed["comments"][1];
    assert!(entry.get("hidden").is_none());
    let row = entry["cell"]["row"].as_object().unwrap();
    let keys: Vec<&String> = row.keys().collect();
    assert_eq!(keys, vec!["B2", "D2", "E2"], "hidden column C is left out");

    std::fs::remove_dir_all(&dir).ok();
}

fn dump_of(file: &str, sheet: Option<&str>) -> String {
    let path = fixture(file);
    let mut args = vec!["dump", path.to_str().unwrap()];
    if let Some(name) = sheet {
        args.extend(["--sheet", name]);
    }
    let out = docrev().args(&args).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn dump_starts_on_the_first_shown_sheet_and_can_still_name_a_hidden_one() {
    let text = dump_of("hidden_first.xlsx", None);
    assert!(text.starts_with("Sheet: 本体 (2/2)"), "{text}");
    assert!(text.contains("second"), "{text}");
    let text = dump_of("hidden.xlsx", Some("作業用"));
    assert!(text.contains("scratch"), "{text}");
}

#[test]
fn a_workbook_that_hides_every_sheet_still_opens() {
    let text = dump_of("all_hidden.xlsx", None);
    assert!(text.starts_with("Sheet: a (1/2)"), "{text}");
    let document = XlsxSource.load(&fixture("all_hidden.xlsx")).unwrap();
    assert!(document.sheets().iter().all(|s| !s.is_hidden()));
}

#[test]
fn a_thread_on_a_hidden_sheet_is_reachable_and_marked() {
    let dir = std::env::temp_dir().join(format!("docrev-hidden-sheet-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let document = dir.join("hidden.xlsx");
    std::fs::copy(fixture("hidden.xlsx"), &document).unwrap();
    let path = document.to_str().unwrap();

    let add = docrev()
        .args([
            "comment",
            "add",
            path,
            "--cell",
            "作業用!A1",
            "--body",
            "scratch?",
        ])
        .output()
        .unwrap();
    assert!(
        add.status.success(),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );
    let text = docrev().args(["comment", "list", path]).output().unwrap();
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(
        text.contains("作業用!A1 (hidden) [agent] scratch?"),
        "{text}"
    );
    let json = docrev()
        .args(["comment", "list", "--json", path])
        .output()
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(parsed["comments"][0]["hidden"], true);
    assert_eq!(parsed["comments"][0]["cell"]["value"], "scratch");

    std::fs::remove_dir_all(&dir).ok();
}
