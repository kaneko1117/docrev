use std::path::PathBuf;
use std::process::Command;

use docrev::adapter::xlsx_source::XlsxSource;
use docrev::app::ports::DocumentSource;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/comments.xlsx")
}

#[test]
fn both_comment_formats_load_with_authors_and_anchors() {
    let document = XlsxSource.load(&fixture()).unwrap();
    let sheet = &document.sheets()[0];
    let comments = sheet.workbook_comments();
    assert_eq!(comments.len(), 2, "one note + one thread: {comments:?}");

    let note = &comments[0];
    assert_eq!((note.row, note.col), (0, 0), "the legacy note sits on A1");
    assert_eq!(note.author, "田中");
    assert_eq!(note.body, "昔のメモの続き", "text runs concatenate");
    assert!(!note.resolved, "notes have no resolved state");
    assert!(note.replies.is_empty());

    let thread = &comments[1];
    assert_eq!((thread.row, thread.col), (1, 1), "the thread sits on B2");
    assert_eq!(thread.author, "佐藤", "resolved through persons.xml");
    assert_eq!(thread.body, "これ確認して");
    assert!(thread.resolved, "done=1 maps to resolved");
    assert_eq!(thread.replies.len(), 1);
    assert_eq!(thread.replies[0].author, "鈴木");
    assert_eq!(thread.replies[0].body, "対応済みです");
}

/// Excel writes a legacy fallback note onto every threaded-comment cell;
/// showing both would duplicate the conversation.
#[test]
fn the_legacy_fallback_under_a_thread_is_deduplicated() {
    let document = XlsxSource.load(&fixture()).unwrap();
    let sheet = &document.sheets()[0];
    assert!(
        sheet
            .workbook_comments()
            .iter()
            .all(|c| !c.body.contains("[Threaded comment]")),
        "the fallback must not surface"
    );
}

#[test]
fn the_cli_lists_workbook_comments_in_their_own_readonly_array() {
    let doc = fixture();
    let out = Command::new(env!("CARGO_BIN_EXE_docrev"))
        .args(["comment", "list"])
        .arg(&doc)
        .args(["--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let listed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        listed["comments"].as_array().unwrap().len(),
        0,
        "no docrev threads — the arrays never mix"
    );
    let workbook = listed["workbook_comments"].as_array().unwrap();
    assert_eq!(workbook.len(), 2);
    assert_eq!(workbook[0]["anchor"]["sheet"], "レビュー");
    assert_eq!(workbook[0]["anchor"]["cell"], "A1");
    assert_eq!(workbook[0]["author"], "田中");
    assert_eq!(workbook[1]["anchor"]["cell"], "B2");
    assert_eq!(workbook[1]["resolved"], true);
    assert_eq!(workbook[1]["replies"][0]["author"], "鈴木");
}

#[test]
fn unresolved_filter_applies_to_workbook_comments_too() {
    let doc = fixture();
    let out = Command::new(env!("CARGO_BIN_EXE_docrev"))
        .args(["comment", "list"])
        .arg(&doc)
        .args(["--json", "--unresolved"])
        .output()
        .unwrap();
    let listed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let workbook = listed["workbook_comments"].as_array().unwrap();
    assert_eq!(workbook.len(), 1, "the resolved thread drops out");
    assert_eq!(workbook[0]["anchor"]["cell"], "A1");
}
