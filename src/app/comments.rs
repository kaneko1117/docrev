use std::path::Path;

use crate::domain::anchor::Anchor;
use crate::domain::comment::CommentThread;
use crate::domain::document::Document;
use crate::domain::workbook_comment::WorkbookComment;

use super::error::{CommentError, DocumentError, StoreError};
use super::ports::{CommentStore, DocumentSource};

#[derive(Debug, Default)]
pub struct Filter<'a> {
    pub unresolved_only: bool,
    pub author: Option<&'a str>,
    pub sheet: Option<&'a str>,
}

/// What the anchored cell says, derived from the workbook at list time so an
/// agent can act on a batch of threads without reading the sheets.
#[derive(Debug, Clone, PartialEq)]
pub struct CellContext {
    /// The anchored cell's displayed text; a merged anchor shows its region's.
    pub value: String,
    /// The row's other non-empty cells as (A1 ref, displayed text), in
    /// column order — a spec sheet reads row-wise.
    pub row: Vec<(String, String)>,
}

/// A runaway guard for hostile row widths, not a real limit.
const ROW_CONTEXT_CAP: usize = 100;

pub fn list(
    store: &impl CommentStore,
    filter: &Filter,
) -> Result<Vec<CommentThread>, CommentError> {
    Ok(store
        .load()?
        .into_iter()
        .filter(|t| !(filter.unresolved_only && t.resolved))
        .filter(|t| filter.author.is_none_or(|a| t.author == a))
        .filter(|t| filter.sheet.is_none_or(|s| t.anchor.sheet() == s))
        .collect())
}

/// `list` plus everything an agent needs from the workbook: each thread's
/// cell content, and the workbook's own comments (read-only, no id — kept
/// apart so a mis-targeted reply is structurally impossible).
pub struct ContextualList {
    pub threads: Vec<(CommentThread, Option<CellContext>)>,
    /// (sheet name, comment), filtered like the threads.
    pub workbook: Vec<(String, WorkbookComment)>,
}

/// The workbook is context, not the point: any failure to read it (corrupt
/// zip, renamed sheet) degrades to threads without a context and no
/// workbook comments, never an error.
pub fn list_with_context(
    source: &impl DocumentSource,
    store: &impl CommentStore,
    document_path: &Path,
    filter: &Filter,
) -> Result<ContextualList, CommentError> {
    let threads = list(store, filter)?;
    let document: Option<Document> = source.load(document_path).ok();
    let threads = threads
        .into_iter()
        .map(|thread| {
            let context = document
                .as_ref()
                .and_then(|d| cell_context(d, &thread.anchor));
            (thread, context)
        })
        .collect();
    let workbook = document
        .iter()
        .flat_map(Document::sheets)
        .filter(|sheet| filter.sheet.is_none_or(|s| sheet.name() == s))
        .flat_map(|sheet| {
            sheet
                .workbook_comments()
                .iter()
                .map(|c| (sheet.name().to_string(), c.clone()))
        })
        .filter(|(_, c)| !(filter.unresolved_only && c.resolved))
        .filter(|(_, c)| filter.author.is_none_or(|a| c.author == a))
        .collect();
    Ok(ContextualList { threads, workbook })
}

fn cell_context(document: &Document, anchor: &Anchor) -> Option<CellContext> {
    let Anchor::Cell { sheet, row, col } = anchor;
    let index = document.index_of(sheet)?;
    let sheet = &document.sheets()[index];
    let (row, col) = (*row as usize, *col as usize);
    let value = sheet.display_cell(row, col).display_text();
    // `value` already speaks for the whole anchored region, so every cell of
    // it is excluded from the siblings; raw cells (not display) keep other
    // merges from repeating their anchor value across the row. The lookup is
    // loop-invariant — hoisted, or a merge-heavy row scans per column.
    let anchor_merge = sheet.merge_at(row, col);
    let in_anchor_region = |c: usize| match anchor_merge {
        Some(merge) => merge.contains(row, c),
        None => c == col,
    };
    let siblings = (0..sheet.row_len(row))
        .filter(|&c| !in_anchor_region(c))
        .filter_map(|c| {
            let text = sheet.cell(row, c).display_text();
            (!text.is_empty()).then(|| (Anchor::cell("", row as u32, c as u32).cell_ref(), text))
        })
        .take(ROW_CONTEXT_CAP)
        .collect();
    Some(CellContext {
        value,
        row: siblings,
    })
}

/// `target` is `"Sheet!B3"`. The sheet must exist in the document; the cell
/// may lie outside the used range.
pub fn add(
    source: &impl DocumentSource,
    store: &mut impl CommentStore,
    document_path: &Path,
    target: &str,
    body: &str,
    author: &str,
) -> Result<CommentThread, CommentError> {
    let Some(anchor) = Anchor::parse_ref(target) else {
        return Err(CommentError::BadReference(target.to_string()));
    };
    let document = source.load(document_path).map_err(DocumentError::from)?;
    if document.index_of(anchor.sheet()).is_none() {
        return Err(CommentError::Document(DocumentError::SheetNotFound {
            name: anchor.sheet().to_string(),
            available: document.sheet_names().map(str::to_string).collect(),
        }));
    }
    Ok(store.add_thread(anchor, body, author)?)
}

pub fn reply(
    store: &mut impl CommentStore,
    thread_id: &str,
    body: &str,
    author: &str,
) -> Result<CommentThread, CommentError> {
    Ok(store.add_reply(thread_id, body, author)?)
}

pub fn resolve(
    store: &mut impl CommentStore,
    thread_id: &str,
) -> Result<CommentThread, CommentError> {
    store.resolve(thread_id)?;
    store
        .load()?
        .into_iter()
        .find(|t| t.id == thread_id)
        .ok_or_else(|| CommentError::Store(StoreError(format!("no thread with id {thread_id}"))))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::app::error::LoadError;
    use crate::domain::cell::CellValue;
    use crate::domain::document::Document;
    use crate::domain::sheet::Sheet;

    struct FakeSource;

    impl DocumentSource for FakeSource {
        fn load(&self, _: &Path) -> Result<Document, LoadError> {
            Ok(Document::new(vec![
                Sheet::new("売上", vec![vec![CellValue::Number(1.0)]]),
                Sheet::new("経費", vec![vec![CellValue::Number(1.0)]]),
            ]))
        }
    }

    #[derive(Default)]
    struct MemoryStore {
        threads: Vec<CommentThread>,
    }

    impl CommentStore for MemoryStore {
        fn load(&self) -> Result<Vec<CommentThread>, StoreError> {
            Ok(self.threads.clone())
        }
        fn add_thread(
            &mut self,
            anchor: Anchor,
            body: &str,
            author: &str,
        ) -> Result<CommentThread, StoreError> {
            let thread = CommentThread {
                id: format!("t{}", self.threads.len()),
                anchor,
                author: author.into(),
                body: body.into(),
                created_at: "2026-08-12T00:00:00Z".into(),
                resolved: false,
                replies: Vec::new(),
            };
            self.threads.push(thread.clone());
            Ok(thread)
        }
        fn add_reply(
            &mut self,
            thread_id: &str,
            body: &str,
            author: &str,
        ) -> Result<CommentThread, StoreError> {
            let Some(thread) = self.threads.iter_mut().find(|t| t.id == thread_id) else {
                return Err(StoreError(format!("no thread with id {thread_id}")));
            };
            thread.replies.push(crate::domain::comment::Reply {
                id: "r".into(),
                author: author.into(),
                body: body.into(),
                created_at: "2026-08-12T00:00:00Z".into(),
            });
            Ok(thread.clone())
        }
        fn resolve(&mut self, thread_id: &str) -> Result<(), StoreError> {
            let Some(thread) = self.threads.iter_mut().find(|t| t.id == thread_id) else {
                return Err(StoreError(format!("no thread with id {thread_id}")));
            };
            thread.resolved = true;
            Ok(())
        }
    }

    fn path() -> PathBuf {
        PathBuf::from("x.xlsx")
    }

    #[test]
    fn add_validates_reference_and_sheet() {
        let mut store = MemoryStore::default();
        let err = add(&FakeSource, &mut store, &path(), "nope", "b", "agent").unwrap_err();
        assert!(err.to_string().contains("invalid cell reference"));

        let err = add(&FakeSource, &mut store, &path(), "架空!B3", "b", "agent").unwrap_err();
        assert!(err.to_string().contains("架空"), "{err}");
        assert!(err.to_string().contains("売上, 経費"), "{err}");

        let thread = add(&FakeSource, &mut store, &path(), "売上!B3", "b", "claude").unwrap();
        assert_eq!(thread.anchor.cell_ref(), "B3");
        assert_eq!(thread.author, "claude");
    }

    #[test]
    fn list_applies_filters() {
        let mut store = MemoryStore::default();
        add(&FakeSource, &mut store, &path(), "売上!A1", "a", "user").unwrap();
        add(&FakeSource, &mut store, &path(), "経費!A1", "b", "agent").unwrap();
        let resolved = add(&FakeSource, &mut store, &path(), "売上!B2", "c", "user").unwrap();
        resolve(&mut store, &resolved.id).unwrap();

        let all = list(&store, &Filter::default()).unwrap();
        assert_eq!(all.len(), 3);

        let unresolved = list(
            &store,
            &Filter {
                unresolved_only: true,
                ..Filter::default()
            },
        )
        .unwrap();
        assert_eq!(unresolved.len(), 2);

        let by_sheet = list(
            &store,
            &Filter {
                sheet: Some("経費"),
                ..Filter::default()
            },
        )
        .unwrap();
        assert_eq!(by_sheet.len(), 1);

        let by_author = list(
            &store,
            &Filter {
                author: Some("user"),
                ..Filter::default()
            },
        )
        .unwrap();
        assert_eq!(by_author.len(), 2);
    }

    struct RichSource;

    impl DocumentSource for RichSource {
        fn load(&self, _: &Path) -> Result<Document, LoadError> {
            use crate::domain::sheet::MergedRange;
            let sheet = Sheet::new(
                "IT-01",
                vec![
                    vec![
                        CellValue::Text("項番".into()),
                        CellValue::Text("手順".into()),
                        CellValue::Text("期待結果".into()),
                    ],
                    vec![
                        CellValue::Text("IT-01-05".into()),
                        CellValue::Empty,
                        CellValue::Text("ロックの旨が表示される".into()),
                        CellValue::Number(3.0),
                    ],
                    vec![
                        CellValue::Text("結合の値".into()),
                        CellValue::Empty,
                        CellValue::Text("隣".into()),
                    ],
                ],
            )
            .with_merges(vec![MergedRange {
                start_row: 2,
                start_col: 0,
                end_row: 2,
                end_col: 1,
            }]);
            Ok(Document::new(vec![sheet]))
        }
    }

    struct BrokenSource;

    impl DocumentSource for BrokenSource {
        fn load(&self, _: &Path) -> Result<Document, LoadError> {
            Err(LoadError("corrupt zip".into()))
        }
    }

    fn thread_at(store: &mut MemoryStore, target: &str) -> CommentThread {
        add(&RichSource, store, &path(), target, "b", "user").unwrap()
    }

    #[test]
    fn list_with_context_carries_the_cell_and_its_row() {
        let mut store = MemoryStore::default();
        thread_at(&mut store, "IT-01!C2");
        let items = list_with_context(&RichSource, &store, &path(), &Filter::default()).unwrap();
        let (_, context) = &items.threads[0];
        let context = context.as_ref().unwrap();
        assert_eq!(context.value, "ロックの旨が表示される");
        assert_eq!(
            context.row,
            vec![
                ("A2".to_string(), "IT-01-05".to_string()),
                ("D2".to_string(), "3".to_string()),
            ],
            "siblings only, empties dropped, column order"
        );
    }

    #[test]
    fn a_merged_anchor_resolves_to_its_regions_value() {
        let mut store = MemoryStore::default();
        // B3 is the covered cell of the A3:B3 merge
        thread_at(&mut store, "IT-01!B3");
        let items = list_with_context(&RichSource, &store, &path(), &Filter::default()).unwrap();
        let context = items.threads[0].1.as_ref().unwrap();
        assert_eq!(
            context.value, "結合の値",
            "the region's value, like the viewer"
        );
        assert_eq!(
            context.row,
            vec![("C3".to_string(), "隣".to_string())],
            "the covered cell is not repeated as a sibling"
        );
    }

    #[test]
    fn a_cell_outside_the_used_range_yields_an_empty_context() {
        let mut store = MemoryStore::default();
        thread_at(&mut store, "IT-01!H99");
        let items = list_with_context(&RichSource, &store, &path(), &Filter::default()).unwrap();
        let context = items.threads[0].1.as_ref().unwrap();
        assert_eq!(context.value, "");
        assert!(context.row.is_empty());
    }

    #[test]
    fn a_renamed_sheet_or_broken_workbook_degrades_to_no_context() {
        let mut store = MemoryStore::default();
        let mut thread = thread_at(&mut store, "IT-01!A1");
        // simulate a rename after the comment was written
        thread.anchor = Anchor::cell("改名済み", 0, 0);
        store.threads[0] = thread;
        let items = list_with_context(&RichSource, &store, &path(), &Filter::default()).unwrap();
        assert!(
            items.threads[0].1.is_none(),
            "unknown sheet: thread kept, no context"
        );

        let items = list_with_context(&BrokenSource, &store, &path(), &Filter::default()).unwrap();
        assert!(
            items.threads[0].1.is_none(),
            "unreadable workbook never errors"
        );
    }

    #[test]
    fn resolve_returns_the_updated_thread() {
        let mut store = MemoryStore::default();
        let thread = add(&FakeSource, &mut store, &path(), "売上!A1", "b", "agent").unwrap();
        let updated = resolve(&mut store, &thread.id).unwrap();
        assert!(updated.resolved);
        assert!(resolve(&mut store, "bogus").is_err());
    }
}
