use std::path::Path;

use crate::app::error::LoadError;
use crate::app::ports::{DocumentKind, DocumentSource};
use crate::domain::document::Document;

use super::markdown_source::MarkdownSource;
use super::xlsx_source::XlsxSource;

const WORKBOOK: [&str; 4] = ["xlsx", "xlsm", "xltx", "xltm"];
const TEXT: [&str; 2] = ["md", "markdown"];

/// Picks the reader by extension, case-insensitively; anything else is `LoadError::Unsupported`.
pub struct ByExtension;

impl ByExtension {
    fn source_for(path: &Path) -> Result<&'static dyn DocumentSource, LoadError> {
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        if WORKBOOK.contains(&extension.as_str()) {
            Ok(&XlsxSource)
        } else if TEXT.contains(&extension.as_str()) {
            Ok(&MarkdownSource)
        } else {
            Err(LoadError::Unsupported {
                extension: format!(".{extension}"),
                supported: WORKBOOK
                    .iter()
                    .chain(TEXT.iter())
                    .map(|e| format!(".{e}"))
                    .collect(),
            })
        }
    }
}

impl DocumentSource for ByExtension {
    fn load(&self, path: &Path) -> Result<Document, LoadError> {
        Self::source_for(path)?.load(path)
    }

    fn revision(&self, path: &Path) -> Option<u64> {
        Self::source_for(path).ok()?.revision(path)
    }

    fn kind(&self, path: &Path) -> DocumentKind {
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        if TEXT.contains(&extension.as_str()) {
            DocumentKind::Text
        } else {
            DocumentKind::Workbook
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_extensions_open_as_text_regardless_of_case() {
        for name in [
            "notes.md",
            "notes.MD",
            "notes.markdown",
            "dir.xlsx/notes.Markdown",
        ] {
            let path =
                std::env::temp_dir().join(format!("docrev-ext-{}-{name}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "one\ntwo\n").unwrap();
            let document = ByExtension.load(&path).unwrap();
            assert_eq!(document.text().map(|t| t.len()), Some(2), "{name}");
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn workbook_extensions_go_to_the_xlsx_reader_and_others_are_unsupported() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/basic.xlsx");
        assert!(ByExtension.load(&fixture).unwrap().workbook().is_some());
        assert_eq!(ByExtension.kind(&fixture), DocumentKind::Workbook);
        assert_eq!(ByExtension.kind(Path::new("x.MD")), DocumentKind::Text);
        for name in ["plain.txt", "noext", "sheet.XLSM.bak"] {
            let err = ByExtension.load(Path::new(name)).unwrap_err();
            assert!(
                matches!(err, LoadError::Unsupported { .. }),
                "{name}: {err:?}"
            );
            assert!(
                err.to_string()
                    .contains(".xlsx, .xlsm, .xltx, .xltm, .md, .markdown"),
                "{err}"
            );
            assert_eq!(ByExtension.revision(Path::new(name)), None, "{name}");
        }
        assert!(
            ByExtension
                .load(Path::new("noext"))
                .unwrap_err()
                .to_string()
                .contains("\".\"")
        );
    }
}
