use std::path::PathBuf;

use docrev::adapter::json_comment_store::JsonCommentStore;
use docrev::adapter::xlsx_source::XlsxSource;
use docrev::app::ports::CommentStore;
use docrev::app::viewer::Viewer;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn loads_threads_from_committed_fixture() {
    let store = JsonCommentStore::for_document(&fixture("basic.xlsx"));
    let threads = store.load().unwrap();
    assert_eq!(threads.len(), 2);
    assert_eq!(threads[0].anchor.cell_ref(), "B2");
    assert_eq!(threads[0].replies[0].author, "claude");
    assert!(threads[1].resolved);
}

#[test]
fn viewer_marks_only_unresolved_threads() {
    let store = JsonCommentStore::for_document(&fixture("basic.xlsx"));
    let viewer = Viewer::open(
        Box::new(XlsxSource),
        Box::new(store),
        &fixture("basic.xlsx"),
    )
    .unwrap();
    assert_eq!(viewer.notice(), None);
    assert_eq!(viewer.unresolved_on_active_sheet(), vec![(1, 1)]);
}
