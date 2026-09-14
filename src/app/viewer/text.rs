use crate::domain::text_document::TextDocument;

use super::add_clamped;

/// A text document with the cursor line; an empty document keeps the cursor at 0.
pub(super) struct TextState {
    document: TextDocument,
    line: usize,
}

impl TextState {
    pub(super) fn new(document: TextDocument) -> Self {
        Self { document, line: 0 }
    }

    pub(super) fn document(&self) -> &TextDocument {
        &self.document
    }

    /// 0-based.
    pub(super) fn line(&self) -> usize {
        self.line
    }

    pub(super) fn last(&self) -> usize {
        self.document.len().saturating_sub(1)
    }

    /// Clamped to the last line.
    pub(super) fn set_line(&mut self, line: usize) {
        self.line = line.min(self.last());
    }

    pub(super) fn step(&mut self, delta: isize) {
        self.line = add_clamped(self.line, delta, self.last());
    }

    /// The cursor keeps its line number, clamped when the file shrank.
    pub(super) fn replace(&mut self, document: TextDocument) {
        self.document = document;
        self.set_line(self.line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: usize) -> TextState {
        TextState::new(TextDocument::new(&"x\n".repeat(lines)))
    }

    #[test]
    fn the_cursor_stays_within_the_file() {
        let mut t = text(3);
        t.step(5);
        assert_eq!(t.line(), 2);
        t.step(-5);
        assert_eq!(t.line(), 0);
        t.set_line(9);
        assert_eq!(t.line(), 2);
        assert_eq!(t.last(), 2);
    }

    #[test]
    fn an_empty_document_keeps_the_cursor_at_zero() {
        let mut t: TextState = text(0);
        t.step(1);
        t.set_line(4);
        assert_eq!(t.line(), 0);
        assert_eq!(t.last(), 0);
        assert!(t.document().is_empty());
    }

    #[test]
    fn replace_keeps_the_line_and_clamps_when_the_file_shrank() {
        let mut t = text(5);
        t.set_line(4);
        t.replace(TextDocument::new("a\nb\nc\nd\ne\nf\n"));
        assert_eq!(t.line(), 4, "a longer file keeps the line");
        t.replace(TextDocument::new("a\nb\n"));
        assert_eq!(t.line(), 1, "a shorter file clamps it");
        assert_eq!(t.document().line(1), Some("b"));
    }
}
