use std::path::Path;

use crate::domain::anchor::Anchor;
use crate::domain::cell::CellValue;
use crate::domain::comment::CommentThread;
use crate::domain::document::{Document, Workbook};
use crate::domain::sheet::{MergedRange, Sheet};
use crate::domain::workbook_comment::WorkbookComment;

use super::error::{CommentError, DocumentError, StoreError};
use super::ports::{CommentStore, DocumentSource};

#[derive(Debug, Default)]
pub struct Filter<'a> {
    pub unresolved_only: bool,
    pub author: Option<&'a str>,
    pub sheet: Option<&'a str>,
}

/// The machine-readable value behind a formatted display.
#[derive(Debug, Clone, PartialEq)]
pub enum RawValue {
    /// `YYYY-MM-DD HH:MM:SS`, a bare time, or elapsed time, as the cell states it.
    DateTime(String),
    /// The stored number a number format rendered.
    Number(f64),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CellContext {
    /// A merged anchor shows its region's text.
    pub value: String,
    /// `None` unless a format produced the display.
    pub raw: Option<RawValue>,
    /// (A1 ref, displayed text) in column order; hidden columns are left out, and a hidden row
    /// has none.
    pub row: Vec<(String, String)>,
    /// Nothing of the anchored cell is on screen: a hidden row, column or sheet.
    pub hidden: bool,
}

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
        .filter(|t| filter.sheet.is_none_or(|s| t.anchor.sheet() == Some(s)))
        .collect())
}

pub struct ContextualList {
    pub threads: Vec<(CommentThread, Option<CellContext>)>,
    /// (sheet name, comment).
    pub workbook: Vec<(String, WorkbookComment)>,
}

/// A workbook that cannot be read yields threads without context, never an error.
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
        .filter_map(Document::workbook)
        .flat_map(Workbook::sheets)
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
    let Anchor::Cell { sheet, row, col } = anchor else {
        return None;
    };
    let workbook = document.workbook()?;
    let index = workbook.index_of(sheet)?;
    let sheet = &workbook.sheets()[index];
    let (row, col) = (*row as usize, *col as usize);
    let display = sheet.display_cell(row, col);
    let value = display.display_text();
    let raw = match display {
        CellValue::DateTime { raw, .. } => Some(RawValue::DateTime(raw.clone())),
        // a hostile file can carry NaN or inf, which JSON cannot hold
        CellValue::FormattedNumber { value, .. } if value.is_finite() => {
            Some(RawValue::Number(*value))
        }
        _ => None,
    };
    // hoisted: a per-column merge lookup scans the merge list each time
    let anchor_merge = sheet.merge_at(row, col);
    let in_anchor_region = |c: usize| match anchor_merge {
        Some(merge) => merge.contains(row, c),
        None => c == col,
    };
    // a merged anchor may sit on a hidden row while its region shows further down: the row a
    // person sees is the region's first shown one
    let shown = sheet.shown_cell_of(row, col).filter(|_| !sheet.is_hidden());
    let hidden = shown.is_none();
    let sibling_row = shown.map_or(row, |(r, _)| r);
    let visible_len = if hidden {
        0
    } else {
        sheet.row_len(sibling_row)
    };
    let siblings = (0..visible_len)
        .filter(|&c| !in_anchor_region(c) && !sheet.col_hidden(c))
        .filter_map(|c| {
            let text = sheet.cell(sibling_row, c).display_text();
            (!text.is_empty()).then(|| (Anchor::a1(sibling_row as u32, c as u32), text))
        })
        .take(ROW_CONTEXT_CAP)
        .collect();
    Some(CellContext {
        value,
        raw,
        row: siblings,
        hidden,
    })
}

/// The cell's conversation: its first unresolved thread, else its newest; a merged region
/// counts as one cell.
pub(crate) fn thread_on<'a>(
    threads: &'a [CommentThread],
    sheet: &Sheet,
    row: usize,
    col: usize,
) -> Option<&'a CommentThread> {
    let merge = sheet.merge_at(row, col);
    let in_region = |r: usize, c: usize| match merge {
        Some(m) => m.contains(r, c),
        None => (r, c) == (row, col),
    };
    let mut newest = None;
    for thread in threads {
        let Anchor::Cell {
            sheet: name,
            row: r,
            col: c,
        } = &thread.anchor
        else {
            continue;
        };
        if name != sheet.name() || !in_region(*r as usize, *c as usize) {
            continue;
        }
        if !thread.resolved {
            return Some(thread);
        }
        newest = Some(thread);
    }
    newest
}

/// Continues the cell's thread, else starts one anchored at the cell (a merged region's
/// top-left); the choice is made by the store at write time.
pub(crate) fn comment_on_cell(
    store: &mut dyn CommentStore,
    sheet: &Sheet,
    row: usize,
    col: usize,
    body: &str,
    author: &str,
) -> Result<CommentThread, StoreError> {
    let (anchor_row, anchor_col) = sheet
        .merge_at(row, col)
        .map_or((row, col), MergedRange::anchor);
    let anchor = Anchor::cell(sheet.name(), anchor_row as u32, anchor_col as u32);
    let existing =
        |threads: &[CommentThread]| thread_on(threads, sheet, row, col).map(|t| t.id.clone());
    store.comment_on(anchor, &existing, body, author)
}

/// The line's conversation: its first unresolved thread, else its newest.
pub fn thread_on_line(threads: &[CommentThread], line: u32) -> Option<&CommentThread> {
    let mut newest = None;
    for thread in threads {
        if thread.anchor != Anchor::line(line) {
            continue;
        }
        if !thread.resolved {
            return Some(thread);
        }
        newest = Some(thread);
    }
    newest
}

/// Continues the line's thread, else starts one; the choice is made by the store at write time.
pub fn comment_on_line(
    store: &mut dyn CommentStore,
    line: u32,
    body: &str,
    author: &str,
) -> Result<CommentThread, StoreError> {
    let existing = |threads: &[CommentThread]| thread_on_line(threads, line).map(|t| t.id.clone());
    store.comment_on(Anchor::line(line), &existing, body, author)
}

/// The sheet must exist; the cell may lie outside the used range. A cell that already has a
/// thread gets the body as a reply on it, which reopens a resolved one.
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
    let Anchor::Cell { sheet, row, col } = &anchor else {
        return Err(CommentError::BadReference(target.to_string()));
    };
    let document = source.load(document_path).map_err(DocumentError::from)?;
    let workbook = document.workbook().ok_or(DocumentError::NotAWorkbook)?;
    let Some(sheet) = workbook.sheets().iter().find(|s| s.name() == sheet) else {
        return Err(CommentError::Document(DocumentError::SheetNotFound {
            name: sheet.to_string(),
            available: workbook.sheet_names().map(str::to_string).collect(),
        }));
    };
    let (row, col) = (*row as usize, *col as usize);
    Ok(comment_on_cell(store, sheet, row, col, body, author)?)
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
        .ok_or_else(|| CommentError::Store(StoreError::ThreadNotFound(thread_id.to_string())))
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
            Ok(Document::from_sheets(vec![
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
                return Err(StoreError::ThreadNotFound(thread_id.to_string()));
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
                return Err(StoreError::ThreadNotFound(thread_id.to_string()));
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
        assert_eq!(thread.anchor.position(), "B3");
        assert_eq!(thread.author, "claude");
    }

    struct MergedSource;

    impl DocumentSource for MergedSource {
        fn load(&self, _: &Path) -> Result<Document, LoadError> {
            Ok(Document::from_sheets(vec![
                Sheet::new("売上", vec![vec![CellValue::Number(1.0); 3]]).with_merges(vec![
                    MergedRange {
                        start_row: 0,
                        start_col: 0,
                        end_row: 0,
                        end_col: 2,
                    },
                ]),
            ]))
        }
    }

    #[test]
    fn add_on_a_commented_cell_continues_its_thread() {
        let mut store = MemoryStore::default();
        let first = add(&FakeSource, &mut store, &path(), "売上!B3", "first", "user").unwrap();
        resolve(&mut store, &first.id).unwrap();

        let again = add(
            &FakeSource,
            &mut store,
            &path(),
            "売上!B3",
            "again",
            "claude",
        )
        .unwrap();
        assert_eq!(again.id, first.id, "the reply lands on the existing thread");
        assert_eq!(again.replies.len(), 1);
        assert_eq!(again.replies[0].body, "again");
        assert_eq!(store.threads.len(), 1, "no second thread on the cell");

        let elsewhere = add(&FakeSource, &mut store, &path(), "売上!C3", "other", "user").unwrap();
        assert_ne!(elsewhere.id, first.id);
        assert_eq!(store.threads.len(), 2);
    }

    #[test]
    fn add_inside_a_merged_region_joins_the_regions_thread() {
        let mut store = MemoryStore::default();
        let first = add(
            &MergedSource,
            &mut store,
            &path(),
            "売上!B1",
            "first",
            "user",
        )
        .unwrap();
        assert_eq!(
            first.anchor.position(),
            "A1",
            "anchored at the region's top-left"
        );

        let again = add(
            &MergedSource,
            &mut store,
            &path(),
            "売上!C1",
            "again",
            "user",
        )
        .unwrap();
        assert_eq!(again.id, first.id);
        assert_eq!(store.threads.len(), 1);
    }

    #[test]
    fn thread_on_prefers_the_open_thread_and_falls_back_to_the_newest() {
        let sheet = Sheet::new("s", vec![vec![CellValue::Number(1.0); 2]]);
        let mut store = MemoryStore::default();
        let older = store
            .add_thread(Anchor::cell("s", 0, 0), "older", "user")
            .unwrap();
        let newer = store
            .add_thread(Anchor::cell("s", 0, 0), "newer", "user")
            .unwrap();
        store
            .add_thread(Anchor::cell("s", 0, 1), "elsewhere", "user")
            .unwrap();

        let pick =
            |threads: &[CommentThread]| thread_on(threads, &sheet, 0, 0).map(|t| t.id.clone());
        assert_eq!(
            pick(&store.threads),
            Some(older.id.clone()),
            "first unresolved"
        );

        resolve(&mut store, &older.id).unwrap();
        assert_eq!(
            pick(&store.threads),
            Some(newer.id.clone()),
            "still unresolved"
        );

        resolve(&mut store, &newer.id).unwrap();
        assert_eq!(
            pick(&store.threads),
            Some(newer.id.clone()),
            "all resolved: newest"
        );

        assert_eq!(thread_on(&store.threads, &sheet, 1, 1), None);
    }

    #[test]
    fn thread_on_line_prefers_the_open_thread_and_falls_back_to_the_newest() {
        let mut store = MemoryStore::default();
        let older = store.add_thread(Anchor::line(3), "older", "user").unwrap();
        let newer = store.add_thread(Anchor::line(3), "newer", "user").unwrap();
        store
            .add_thread(Anchor::line(4), "elsewhere", "user")
            .unwrap();

        let pick = |threads: &[CommentThread]| thread_on_line(threads, 3).map(|t| t.id.clone());
        assert_eq!(pick(&store.threads), Some(older.id.clone()));
        resolve(&mut store, &older.id).unwrap();
        assert_eq!(pick(&store.threads), Some(newer.id.clone()));
        resolve(&mut store, &newer.id).unwrap();
        assert_eq!(pick(&store.threads), Some(newer.id.clone()), "newest");
        assert_eq!(thread_on_line(&store.threads, 5), None);
    }

    #[test]
    fn comment_on_line_continues_the_lines_thread_and_reopens_a_resolved_one() {
        let mut store = MemoryStore::default();
        let first = comment_on_line(&mut store, 3, "first", "user").unwrap();
        resolve(&mut store, &first.id).unwrap();
        let again = comment_on_line(&mut store, 3, "again", "agent").unwrap();
        assert_eq!(again.id, first.id, "one line, one thread");
        assert_eq!(again.replies.len(), 1);

        let other = comment_on_line(&mut store, 4, "other", "user").unwrap();
        assert_ne!(other.id, first.id);
        assert_eq!(other.anchor, Anchor::line(4));
        assert_eq!(store.threads.len(), 2);
    }

    #[test]
    fn a_text_document_has_no_cells_to_comment_on() {
        struct TextSource;
        impl DocumentSource for TextSource {
            fn load(&self, _: &Path) -> Result<Document, LoadError> {
                Ok(Document::from_text("a\nb"))
            }
        }
        let mut store = MemoryStore::default();
        let err = add(&TextSource, &mut store, &path(), "s!A1", "x", "user").unwrap_err();
        assert!(
            matches!(err, CommentError::Document(DocumentError::NotAWorkbook)),
            "{err:?}"
        );
        assert!(store.threads.is_empty());
    }

    #[test]
    fn the_sheet_filter_leaves_line_threads_out() {
        let mut store = MemoryStore::default();
        store.add_thread(Anchor::line(0), "l", "user").unwrap();
        add(&FakeSource, &mut store, &path(), "売上!A1", "a", "user").unwrap();
        let on_sheet = list(
            &store,
            &Filter {
                sheet: Some("売上"),
                ..Filter::default()
            },
        )
        .unwrap();
        assert_eq!(on_sheet.len(), 1);
        assert_eq!(on_sheet[0].anchor, Anchor::cell("売上", 0, 0));
        assert_eq!(list(&store, &Filter::default()).unwrap().len(), 2);
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
                    vec![
                        CellValue::DateTime {
                            text: "2026年8月31日(月)".into(),
                            raw: "2026-08-31 00:00:00".into(),
                        },
                        CellValue::FormattedNumber {
                            value: 1_234_000.0,
                            text: "1,234千円".into(),
                        },
                        CellValue::FormattedNumber {
                            value: f64::NAN,
                            text: "NaN%".into(),
                        },
                    ],
                ],
            )
            .with_merges(vec![MergedRange {
                start_row: 2,
                start_col: 0,
                end_row: 2,
                end_col: 1,
            }]);
            Ok(Document::from_sheets(vec![sheet]))
        }
    }

    struct BrokenSource;

    impl DocumentSource for BrokenSource {
        fn load(&self, _: &Path) -> Result<Document, LoadError> {
            Err(LoadError::Open("corrupt zip".into()))
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

    struct HiddenSource;

    impl DocumentSource for HiddenSource {
        fn load(&self, path: &Path) -> Result<Document, LoadError> {
            use std::collections::HashSet;
            let document = RichSource.load(path)?;
            let sheet = document.into_workbook().unwrap().into_sheets().remove(0);
            Ok(Document::from_sheets(vec![
                sheet
                    .with_hidden_cols(HashSet::from([3]))
                    .with_hidden_rows(HashSet::from([3])),
            ]))
        }
    }

    struct MergedHiddenSource;

    impl DocumentSource for MergedHiddenSource {
        fn load(&self, _: &Path) -> Result<Document, LoadError> {
            use crate::domain::sheet::MergedRange;
            use std::collections::HashSet;
            let sheet = Sheet::new(
                "IT-01",
                vec![
                    vec![
                        CellValue::Text("見出し".into()),
                        CellValue::Text("上".into()),
                    ],
                    vec![CellValue::Empty, CellValue::Text("下".into())],
                ],
            )
            .with_merges(vec![MergedRange {
                start_row: 0,
                start_col: 0,
                end_row: 1,
                end_col: 0,
            }])
            .with_hidden_rows(HashSet::from([0]));
            Ok(Document::from_sheets(vec![sheet]))
        }
    }

    #[test]
    fn a_merged_anchor_on_a_hidden_row_is_not_hidden_and_reads_the_shown_row() {
        let mut store = MemoryStore::default();
        thread_at(&mut store, "IT-01!A1");
        let items =
            list_with_context(&MergedHiddenSource, &store, &path(), &Filter::default()).unwrap();
        let context = items.threads[0].1.as_ref().unwrap();
        assert!(!context.hidden, "the region shows on row 2");
        assert_eq!(context.value, "見出し");
        assert_eq!(context.row, vec![("B2".to_string(), "下".to_string())]);
    }

    struct HiddenSheetSource;

    impl DocumentSource for HiddenSheetSource {
        fn load(&self, path: &Path) -> Result<Document, LoadError> {
            let sheet = RichSource
                .load(path)?
                .into_workbook()
                .unwrap()
                .into_sheets()
                .remove(0);
            Ok(Document::from_sheets(vec![
                sheet.with_hidden(true),
                Sheet::new("shown", vec![vec![CellValue::Number(1.0)]]),
            ]))
        }
    }

    #[test]
    fn a_thread_on_a_hidden_sheet_is_flagged_but_still_reads_its_cell() {
        let mut store = MemoryStore::default();
        thread_at(&mut store, "IT-01!C2");
        let items =
            list_with_context(&HiddenSheetSource, &store, &path(), &Filter::default()).unwrap();
        let context = items.threads[0].1.as_ref().unwrap();
        assert!(context.hidden);
        assert_eq!(context.value, "ロックの旨が表示される");
        assert!(context.row.is_empty(), "nothing of a hidden sheet is seen");
    }

    #[test]
    fn hidden_columns_leave_the_row_and_a_hidden_anchor_is_flagged() {
        let mut store = MemoryStore::default();
        thread_at(&mut store, "IT-01!C2");
        let items = list_with_context(&HiddenSource, &store, &path(), &Filter::default()).unwrap();
        let context = items.threads[0].1.as_ref().unwrap();
        assert!(!context.hidden);
        assert_eq!(
            context.row,
            vec![("A2".to_string(), "IT-01-05".to_string())],
            "D2 is hidden and left out"
        );

        let mut store = MemoryStore::default();
        thread_at(&mut store, "IT-01!A4");
        let items = list_with_context(&HiddenSource, &store, &path(), &Filter::default()).unwrap();
        let context = items.threads[0].1.as_ref().unwrap();
        assert!(context.hidden, "row 4 is hidden");
        assert_eq!(
            context.value, "2026年8月31日(月)",
            "the cell itself is still read"
        );
        assert!(context.row.is_empty(), "a hidden row shows no siblings");
    }

    #[test]
    fn a_merged_anchor_resolves_to_its_regions_value() {
        let mut store = MemoryStore::default();
        // B3 is covered by the A3:B3 merge
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
    fn a_date_anchor_carries_its_machine_readable_raw() {
        let mut store = MemoryStore::default();
        thread_at(&mut store, "IT-01!A4");
        let items = list_with_context(&RichSource, &store, &path(), &Filter::default()).unwrap();
        let context = items.threads[0].1.as_ref().unwrap();
        assert_eq!(context.value, "2026年8月31日(月)");
        assert_eq!(
            context.raw,
            Some(RawValue::DateTime("2026-08-31 00:00:00".into()))
        );

        let mut store = MemoryStore::default();
        thread_at(&mut store, "IT-01!C2");
        let items = list_with_context(&RichSource, &store, &path(), &Filter::default()).unwrap();
        let context = items.threads[0].1.as_ref().unwrap();
        assert_eq!(context.raw, None, "plain cells carry no raw");
    }

    #[test]
    fn a_formatted_number_anchor_carries_its_stored_value() {
        let mut store = MemoryStore::default();
        thread_at(&mut store, "IT-01!B4");
        let items = list_with_context(&RichSource, &store, &path(), &Filter::default()).unwrap();
        let context = items.threads[0].1.as_ref().unwrap();
        assert_eq!(context.value, "1,234千円");
        assert_eq!(context.raw, Some(RawValue::Number(1_234_000.0)));
        assert_eq!(
            context.row,
            vec![
                ("A4".into(), "2026年8月31日(月)".into()),
                ("C4".into(), "NaN%".into()),
            ],
            "siblings keep their displayed text"
        );

        let mut store = MemoryStore::default();
        thread_at(&mut store, "IT-01!D2");
        let items = list_with_context(&RichSource, &store, &path(), &Filter::default()).unwrap();
        let context = items.threads[0].1.as_ref().unwrap();
        assert_eq!(context.value, "3");
        assert_eq!(
            context.raw, None,
            "a plain number already is its raw rendering"
        );

        let mut store = MemoryStore::default();
        thread_at(&mut store, "IT-01!C4");
        let items = list_with_context(&RichSource, &store, &path(), &Filter::default()).unwrap();
        let context = items.threads[0].1.as_ref().unwrap();
        assert_eq!(context.raw, None, "a non-finite value has no JSON form");
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
