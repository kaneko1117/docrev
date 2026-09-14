use std::path::Path;

use crate::app::error::LoadError;
use crate::app::ports::{DocumentKind, DocumentSource};
use crate::domain::document::Document;
use crate::domain::text_document::TextDocument;
use crate::infra::{fs, markdown};

/// UTF-8 only; a leading BOM is dropped.
pub struct MarkdownSource;

impl DocumentSource for MarkdownSource {
    fn load(&self, path: &Path) -> Result<Document, LoadError> {
        let bytes = std::fs::read(path)
            .map_err(|e| LoadError::Open(format!("cannot open {}: {e}", path.display())))?;
        let text = String::from_utf8(bytes)
            .map_err(|_| LoadError::Open(format!("{} is not a UTF-8 text file", path.display())))?;
        let source = TextDocument::new(text.strip_prefix('\u{feff}').unwrap_or(&text));
        let shown = markdown::shown_lines(source.lines());
        Ok(Document::Text(source.with_shown(shown)))
    }

    fn revision(&self, path: &Path) -> Option<u64> {
        fs::revision(path)
    }

    fn kind(&self, _path: &Path) -> DocumentKind {
        DocumentKind::Text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::text_document::Face;

    fn temp_file(bytes: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("docrev-md-{}.md", uuid::Uuid::new_v4()));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn loads_lines_and_drops_a_bom() {
        let path = temp_file("\u{feff}# title\r\nbody\n".as_bytes());
        let document = MarkdownSource.load(&path).unwrap();
        let text = document.text().unwrap();
        assert_eq!(text.lines(), ["# title", "body"]);
        assert_eq!(text.shown(0), [("title".to_string(), Face::Heading(1))]);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_non_utf8_file_is_an_open_error() {
        let path = temp_file(&[0x23, 0x20, 0xff, 0xfe, 0x0a]);
        let err = MarkdownSource.load(&path).unwrap_err();
        assert!(matches!(err, LoadError::Open(_)), "{err:?}");
        assert!(err.to_string().contains("not a UTF-8 text file"), "{err}");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_missing_file_is_an_open_error_and_revision_zero() {
        let path = std::env::temp_dir().join("docrev-md-missing.md");
        let err = MarkdownSource.load(&path).unwrap_err();
        assert!(matches!(err, LoadError::Open(_)), "{err:?}");
        assert_eq!(MarkdownSource.revision(&path), Some(0));
    }

    #[test]
    fn revision_changes_with_the_file() {
        let path = temp_file(b"a\n");
        let before = MarkdownSource.revision(&path);
        std::fs::write(&path, b"a\nb\n").unwrap();
        assert_ne!(MarkdownSource.revision(&path), before);
        let _ = std::fs::remove_file(path);
    }
}
