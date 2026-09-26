/// How a run of text is shown; the markup that produced it is not part of the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Face {
    Plain,
    /// 1 to 6.
    Heading(u8),
    Bold,
    Italic,
    Strike,
    Code,
    /// A whole line inside a fenced block, or the fence itself.
    CodeBlock,
    /// The `•`, `☐` or `☑` put in place of a list marker.
    ListMarker,
    Link,
    /// The `>` of a quote, kept but dimmed.
    Quote,
    /// A line of the leading `---` block.
    FrontMatter,
    /// A thematic break, drawn across the whole width whatever its text.
    Rule,
    /// A table's `│` borders and its header separator.
    TableEdge,
}

/// (text as shown, how it is shown).
pub type Run = (String, Face);

/// 0-based lines of a text file; a trailing newline does not open an empty last line.
#[derive(Debug, Clone, PartialEq)]
pub struct TextDocument {
    lines: Vec<String>,
    /// Line `i` as it is shown, one entry per line.
    shown: Vec<Vec<Run>>,
}

impl TextDocument {
    /// `\r` before a newline is dropped; every line is shown as written.
    pub fn new(text: &str) -> Self {
        Self::from_lines(text.lines().map(str::to_string).collect())
    }

    /// Lines as given, each free of line breaks, so their count is the line count.
    pub fn from_lines(lines: Vec<String>) -> Self {
        let shown = lines
            .iter()
            .map(|line| vec![(line.clone(), Face::Plain)])
            .collect();
        Self { lines, shown }
    }

    /// Missing entries are shown as written and extra ones are dropped, so each line keeps one.
    pub fn with_shown(mut self, mut shown: Vec<Vec<Run>>) -> Self {
        shown.truncate(self.lines.len());
        for line in &self.lines[shown.len()..] {
            shown.push(vec![(line.clone(), Face::Plain)]);
        }
        self.shown = shown;
        self
    }

    /// Empty past the last line.
    pub fn shown(&self, index: usize) -> &[Run] {
        self.shown.get(index).map_or(&[][..], Vec::as_slice)
    }

    /// The line as it reads on screen, leaving out rules since they show no text.
    pub fn shown_text(&self, index: usize) -> String {
        self.shown(index)
            .iter()
            .filter(|(_, face)| *face != Face::Rule)
            .map(|(text, _)| text.as_str())
            .collect()
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
    fn every_line_keeps_exactly_one_shown_line() {
        let document = TextDocument::new("# a\nb\n");
        assert_eq!(document.shown(0), [("# a".to_string(), Face::Plain)]);
        let document = document.with_shown(vec![vec![("a".to_string(), Face::Heading(1))]]);
        assert_eq!(document.shown(0), [("a".to_string(), Face::Heading(1))]);
        assert_eq!(
            document.shown(1),
            [("b".to_string(), Face::Plain)],
            "a missing entry is shown as written"
        );
        assert!(document.shown(2).is_empty());
        let extra = TextDocument::new("x\n").with_shown(vec![Vec::new(); 5]);
        assert_eq!(extra.shown.len(), 1, "extra entries are dropped");
    }

    #[test]
    fn shown_text_is_what_reads_on_screen() {
        let document = TextDocument::new("[a](u) **b**\n---\n").with_shown(vec![
            vec![
                ("a".to_string(), Face::Link),
                (" ".to_string(), Face::Plain),
                ("b".to_string(), Face::Bold),
            ],
            vec![("---".to_string(), Face::Rule)],
        ]);
        assert_eq!(document.shown_text(0), "a b");
        assert_eq!(document.shown_text(1), "", "a rule shows no text");
        assert_eq!(document.shown_text(2), "");
    }

    #[test]
    fn lines_given_as_such_keep_their_count() {
        assert_eq!(TextDocument::from_lines(vec![String::new()]).len(), 1);
        assert!(TextDocument::from_lines(Vec::new()).is_empty());
        let document = TextDocument::from_lines(vec!["a".to_string(), String::new()]);
        assert_eq!(document.lines(), ["a", ""]);
        assert_eq!(document.shown(1), [(String::new(), Face::Plain)]);
    }

    #[test]
    fn crlf_endings_leave_no_carriage_return() {
        let document = TextDocument::new("a\r\nb\r\n");
        assert_eq!(document.lines(), ["a", "b"]);
        assert_eq!(document.line(1), Some("b"));
        assert_eq!(document.line(2), None);
    }
}
