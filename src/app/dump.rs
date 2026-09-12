use std::path::Path;

use crate::domain::document::Document;
use crate::domain::sheet::Sheet;
use crate::domain::text_document::TextDocument;

use super::error::DocumentError;
use super::ports::DocumentSource;

#[derive(Debug)]
pub enum DumpView {
    Sheet {
        sheet: Box<Sheet>,
        /// 0-based index within the document.
        position: usize,
        total: usize,
        formulas: bool,
    },
    Text(TextDocument),
}

impl DumpView {
    /// `None` for a text document.
    pub fn sheet(&self) -> Option<&Sheet> {
        match self {
            DumpView::Sheet { sheet, .. } => Some(sheet),
            DumpView::Text(_) => None,
        }
    }

    /// (0-based position, total); `None` for a text document.
    pub fn place(&self) -> Option<(usize, usize)> {
        match self {
            DumpView::Sheet {
                position, total, ..
            } => Some((*position, *total)),
            DumpView::Text(_) => None,
        }
    }
}

/// The first shown sheet when `sheet_name` is `None`; a name may pick a hidden one. A text
/// document takes neither a sheet name nor `formulas`.
pub fn dump(
    source: &impl DocumentSource,
    path: &Path,
    sheet_name: Option<&str>,
    formulas: bool,
) -> Result<DumpView, DocumentError> {
    let document = match source.load(path)? {
        Document::Workbook(workbook) => workbook,
        Document::Text(text) => {
            return if sheet_name.is_some() {
                Err(DocumentError::NotAWorkbook)
            } else if formulas {
                Err(DocumentError::NoFormulas)
            } else {
                Ok(DumpView::Text(text))
            };
        }
    };
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
        None => document.first_shown().unwrap_or(0),
    };
    let sheet = document
        .into_sheets()
        .into_iter()
        .nth(position)
        .ok_or(DocumentError::EmptyDocument)?;
    Ok(DumpView::Sheet {
        sheet: Box::new(sheet),
        position,
        total,
        formulas,
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
            self.0.clone().map_err(LoadError::Open)
        }
    }

    fn doc(names: &[&str]) -> Document {
        Document::from_sheets(names.iter().map(|n| Sheet::new(*n, vec![])).collect())
    }

    /// (sheet name, position, total, formulas) of a sheet view.
    fn sheet_view(view: DumpView) -> (String, usize, usize, bool) {
        match view {
            DumpView::Sheet {
                sheet,
                position,
                total,
                formulas,
            } => (sheet.name().to_string(), position, total, formulas),
            DumpView::Text(_) => panic!("expected a sheet view"),
        }
    }

    #[test]
    fn defaults_to_first_sheet() {
        let source = FakeSource(Ok(doc(&["one", "two"])));
        let view = dump(&source, Path::new("x"), None, true).unwrap();
        assert_eq!(sheet_view(view), ("one".to_string(), 0, 2, true));
    }

    #[test]
    fn defaults_to_the_first_shown_sheet_and_names_reach_hidden_ones() {
        use crate::domain::cell::CellValue;
        use crate::domain::sheet::Sheet;
        let document = Document::from_sheets(vec![
            Sheet::new("scratch", vec![vec![CellValue::Empty]]).with_hidden(true),
            Sheet::new("main", vec![vec![CellValue::Empty]]),
        ]);
        let source = FakeSource(Ok(document));
        let view = dump(&source, Path::new("x.xlsx"), None, false).unwrap();
        assert_eq!(sheet_view(view).0, "main");
        let view = dump(&source, Path::new("x.xlsx"), Some("scratch"), false).unwrap();
        assert_eq!(sheet_view(view).0, "scratch");
    }

    #[test]
    fn selects_sheet_by_name() {
        let source = FakeSource(Ok(doc(&["one", "two"])));
        let view = dump(&source, Path::new("x"), Some("two"), false).unwrap();
        assert_eq!(sheet_view(view), ("two".to_string(), 1, 2, false));
    }

    #[test]
    fn unknown_sheet_reports_candidates() {
        let source = FakeSource(Ok(doc(&["one", "two"])));
        let err = dump(&source, Path::new("x"), Some("nope"), false).unwrap_err();
        assert!(matches!(err, DocumentError::SheetNotFound { .. }));
        let msg = err.to_string();
        assert!(msg.contains("one, two"), "unexpected message: {msg}");
    }

    #[test]
    fn empty_document_is_an_error() {
        let source = FakeSource(Ok(doc(&[])));
        let err = dump(&source, Path::new("x"), None, false).unwrap_err();
        assert!(matches!(err, DocumentError::EmptyDocument));
    }

    #[test]
    fn load_failure_propagates() {
        let source = FakeSource(Err("boom".to_string()));
        let err = dump(&source, Path::new("x"), None, false).unwrap_err();
        assert!(err.to_string().contains("boom"));
    }

    #[test]
    fn a_text_document_dumps_whole_and_takes_no_sheet_or_formulas() {
        let source = FakeSource(Ok(Document::from_text("a\nb\n")));
        match dump(&source, Path::new("x.md"), None, false).unwrap() {
            DumpView::Text(text) => assert_eq!(text.len(), 2),
            DumpView::Sheet { .. } => panic!("expected a text view"),
        }
        let err = dump(&source, Path::new("x.md"), Some("one"), false).unwrap_err();
        assert!(matches!(err, DocumentError::NotAWorkbook), "{err:?}");
        let err = dump(&source, Path::new("x.md"), None, true).unwrap_err();
        assert!(matches!(err, DocumentError::NoFormulas), "{err:?}");
    }
}
