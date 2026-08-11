use std::path::Path;

use crate::domain::sheet::Sheet;

use super::error::DocumentError;
use super::ports::DocumentSource;

#[derive(Debug)]
pub struct DumpView {
    pub sheet: Sheet,
    /// 0-based index within the document.
    pub position: usize,
    pub total: usize,
}

/// First sheet when `sheet_name` is `None`.
pub fn dump(
    source: &impl DocumentSource,
    path: &Path,
    sheet_name: Option<&str>,
) -> Result<DumpView, DocumentError> {
    let document = source.load(path)?;
    let total = document.sheets().len();
    if total == 0 {
        return Err(DocumentError::EmptyDocument);
    }
    let position = match sheet_name {
        Some(name) => document
            .index_of(name)
            .ok_or_else(|| DocumentError::SheetNotFound {
                name: name.to_string(),
                available: document.sheet_names().map(str::to_string).collect(),
            })?,
        None => 0,
    };
    let sheet = document
        .into_sheets()
        .into_iter()
        .nth(position)
        .ok_or(DocumentError::EmptyDocument)?;
    Ok(DumpView {
        sheet,
        position,
        total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::error::LoadError;
    use crate::domain::document::Document;

    struct FakeSource(Result<Document, String>);

    impl DocumentSource for FakeSource {
        fn load(&self, _: &Path) -> Result<Document, LoadError> {
            self.0.clone().map_err(LoadError)
        }
    }

    fn doc(names: &[&str]) -> Document {
        Document::new(names.iter().map(|n| Sheet::new(*n, vec![])).collect())
    }

    #[test]
    fn defaults_to_first_sheet() {
        let source = FakeSource(Ok(doc(&["one", "two"])));
        let view = dump(&source, Path::new("x"), None).unwrap();
        assert_eq!(view.sheet.name(), "one");
        assert_eq!(view.position, 0);
        assert_eq!(view.total, 2);
    }

    #[test]
    fn selects_sheet_by_name() {
        let source = FakeSource(Ok(doc(&["one", "two"])));
        let view = dump(&source, Path::new("x"), Some("two")).unwrap();
        assert_eq!(view.sheet.name(), "two");
        assert_eq!(view.position, 1);
    }

    #[test]
    fn unknown_sheet_reports_candidates() {
        let source = FakeSource(Ok(doc(&["one", "two"])));
        let err = dump(&source, Path::new("x"), Some("nope")).unwrap_err();
        assert!(matches!(err, DocumentError::SheetNotFound { .. }));
        let msg = err.to_string();
        assert!(msg.contains("one, two"), "unexpected message: {msg}");
    }

    #[test]
    fn empty_document_is_an_error() {
        let source = FakeSource(Ok(doc(&[])));
        let err = dump(&source, Path::new("x"), None).unwrap_err();
        assert!(matches!(err, DocumentError::EmptyDocument));
    }

    #[test]
    fn load_failure_propagates() {
        let source = FakeSource(Err("boom".to_string()));
        let err = dump(&source, Path::new("x"), None).unwrap_err();
        assert!(err.to_string().contains("boom"));
    }
}
