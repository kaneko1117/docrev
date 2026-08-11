use std::path::Path;

use crate::domain::anchor::Anchor;
use crate::domain::comment::CommentThread;

use super::error::{CommentError, DocumentError, StoreError};
use super::ports::{CommentStore, DocumentSource};

#[derive(Debug, Default)]
pub struct Filter<'a> {
    pub unresolved_only: bool,
    pub author: Option<&'a str>,
    pub sheet: Option<&'a str>,
}

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

    #[test]
    fn resolve_returns_the_updated_thread() {
        let mut store = MemoryStore::default();
        let thread = add(&FakeSource, &mut store, &path(), "売上!A1", "b", "agent").unwrap();
        let updated = resolve(&mut store, &thread.id).unwrap();
        assert!(updated.resolved);
        assert!(resolve(&mut store, "bogus").is_err());
    }
}
