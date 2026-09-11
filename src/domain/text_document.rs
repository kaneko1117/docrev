/// 0-based lines of a text file; a trailing newline does not open an empty last line.
#[derive(Debug, Clone, PartialEq)]
pub struct TextDocument {
    lines: Vec<String>,
}

impl TextDocument {
    /// `\r` before a newline is dropped.
    pub fn new(text: &str) -> Self {
        Self {
            lines: text.lines().map(str::to_string).collect(),
        }
    }

    pub fn line(&self, index: usize) -> Option<&str> {
        self.lines.get(index).map(String::as_str)
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trailing_newline_does_not_add_a_line() {
        assert_eq!(TextDocument::new("a\nb").len(), 2);
        assert_eq!(TextDocument::new("a\nb\n").len(), 2);
        assert_eq!(TextDocument::new("a\n\nb\n").lines(), ["a", "", "b"]);
        assert!(TextDocument::new("").is_empty());
    }

    #[test]
    fn crlf_endings_leave_no_carriage_return() {
        let document = TextDocument::new("a\r\nb\r\n");
        assert_eq!(document.lines(), ["a", "b"]);
        assert_eq!(document.line(1), Some("b"));
        assert_eq!(document.line(2), None);
    }
}
