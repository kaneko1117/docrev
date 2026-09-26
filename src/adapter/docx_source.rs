use std::path::Path;

use unicode_width::UnicodeWidthStr;

use crate::app::error::LoadError;
use crate::app::ports::{DocumentKind, DocumentSource};
use crate::domain::document::Document;
use crate::domain::text_document::{Face, Run, TextDocument};
use crate::infra::docx::{self, Block, Emphasis, Paragraph};
use crate::infra::fs;

/// A paragraph or a table row is one line; the text is plain and the file's styling is what is shown.
pub struct DocxSource;

impl DocumentSource for DocxSource {
    fn load(&self, path: &Path) -> Result<Document, LoadError> {
        let blocks = docx::read_blocks(path)
            .map_err(|e| LoadError::Open(format!("cannot open {}: {e}", path.display())))?;
        Ok(Document::Text(text_document(&blocks)))
    }

    fn revision(&self, path: &Path) -> Option<u64> {
        fs::revision(path)
    }

    fn kind(&self, _path: &Path) -> DocumentKind {
        DocumentKind::Text
    }
}

/// A table row's cells are joined by a tab, so a copied row pastes into a spreadsheet as cells.
fn text_document(blocks: &[Block]) -> TextDocument {
    let mut lines = Vec::new();
    let mut shown = Vec::new();
    for block in blocks {
        match block {
            Block::Paragraph(paragraph) => {
                lines.push(one_line(&paragraph.text()));
                shown.push(paragraph_runs(paragraph));
            }
            Block::Table(rows) => {
                let widths = column_widths(rows);
                for row in rows {
                    let cells: Vec<String> = row.iter().map(|cell| cell_text(cell)).collect();
                    lines.push(cells.join("\t"));
                    shown.push(table_row(row, &widths));
                }
            }
        }
    }
    TextDocument::from_lines(lines).with_shown(shown)
}

/// A line holds no line break; everything else stays as written.
fn one_line(text: &str) -> String {
    text.replace(['\n', '\r'], " ")
}

/// A cell holds no tab either, since tabs are what separate the cells.
fn cell_text(cell: &str) -> String {
    one_line(cell).replace('\t', " ")
}

/// Control characters take a space on screen.
fn drawn(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// A heading keeps its face whatever its runs' emphasis, as in Markdown.
fn paragraph_runs(paragraph: &Paragraph) -> Vec<Run> {
    let mut runs = Vec::new();
    if let Some(level) = paragraph.list_level {
        let marker = format!("{}• ", "  ".repeat(usize::from(level)));
        runs.push((marker, Face::ListMarker));
    }
    for (text, emphasis) in &paragraph.spans {
        let face = match paragraph.heading {
            Some(level) => Face::Heading(level),
            None => face(*emphasis),
        };
        runs.push((drawn(text), face));
    }
    runs
}

/// One face per run: a link shows as one whatever else it is, then strike, bold, italic.
fn face(emphasis: Emphasis) -> Face {
    if emphasis.link {
        Face::Link
    } else if emphasis.strike {
        Face::Strike
    } else if emphasis.bold {
        Face::Bold
    } else if emphasis.italic {
        Face::Italic
    } else {
        Face::Plain
    }
}

/// The widest drawn cell of each column over the whole table.
fn column_widths(rows: &[Vec<String>]) -> Vec<usize> {
    let mut widths: Vec<usize> = Vec::new();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            let width = drawn(cell).width();
            match widths.get_mut(i) {
                Some(current) => *current = (*current).max(width),
                None => widths.push(width),
            }
        }
    }
    widths
}

/// Every cell is padded to its column's width, so the rows of a table line up.
fn table_row(row: &[String], widths: &[usize]) -> Vec<Run> {
    let mut runs = vec![("│".to_string(), Face::TableEdge)];
    for (i, width) in widths.iter().enumerate() {
        let text = row.get(i).map(|cell| drawn(cell)).unwrap_or_default();
        let padding = " ".repeat(width.saturating_sub(text.width()));
        runs.push((format!(" {text}{padding} "), Face::Plain));
        runs.push(("│".to_string(), Face::TableEdge));
    }
    runs
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn paragraph(text: &str) -> Block {
        Block::Paragraph(Paragraph {
            spans: vec![(text.to_string(), Emphasis::default())],
            ..Paragraph::default()
        })
    }

    fn strings(cells: &[&str]) -> Vec<String> {
        cells.iter().map(|c| c.to_string()).collect()
    }

    #[test]
    fn a_paragraph_is_a_line_and_a_table_row_is_a_tab_separated_line() {
        let blocks = [
            paragraph("a"),
            Block::Table(vec![strings(&["x", "yy"]), strings(&["長い", "z"])]),
            paragraph("after"),
        ];
        let document = text_document(&blocks);
        assert_eq!(document.lines(), ["a", "x\tyy", "長い\tz", "after"]);
        let edge = ("│".to_string(), Face::TableEdge);
        assert_eq!(
            document.shown(1),
            [
                edge.clone(),
                (" x    ".to_string(), Face::Plain),
                edge.clone(),
                (" yy ".to_string(), Face::Plain),
                edge.clone(),
            ]
        );
        assert_eq!(document.shown_text(2), "│ 長い │ z  │");
    }

    #[test]
    fn a_ragged_row_is_padded_with_empty_cells() {
        let blocks = [Block::Table(vec![strings(&["a", "b"]), strings(&["c"])])];
        let document = text_document(&blocks);
        assert_eq!(document.lines(), ["a\tb", "c"]);
        assert_eq!(document.shown_text(1), "│ c │   │");
    }

    #[test]
    fn headings_lists_and_emphasis_are_shown_from_the_file_not_from_markup() {
        let with = |bold, italic, strike, link| Emphasis {
            bold,
            italic,
            strike,
            link,
        };
        let blocks = [
            Block::Paragraph(Paragraph {
                heading: Some(2),
                spans: vec![
                    ("Title ".to_string(), Emphasis::default()),
                    ("bold".to_string(), with(true, false, false, false)),
                ],
                ..Paragraph::default()
            }),
            Block::Paragraph(Paragraph {
                list_level: Some(1),
                spans: vec![("item".to_string(), Emphasis::default())],
                ..Paragraph::default()
            }),
            Block::Paragraph(Paragraph {
                spans: vec![
                    ("l".to_string(), with(true, false, false, true)),
                    ("s".to_string(), with(true, false, true, false)),
                    ("b".to_string(), with(true, true, false, false)),
                    ("i".to_string(), with(false, true, false, false)),
                    ("p".to_string(), Emphasis::default()),
                ],
                ..Paragraph::default()
            }),
        ];
        let document = text_document(&blocks);
        assert_eq!(document.lines(), ["Title bold", "item", "lsbip"]);
        assert_eq!(
            document.shown(0),
            [
                ("Title ".to_string(), Face::Heading(2)),
                ("bold".to_string(), Face::Heading(2)),
            ]
        );
        assert_eq!(
            document.shown(1),
            [
                ("  • ".to_string(), Face::ListMarker),
                ("item".to_string(), Face::Plain),
            ]
        );
        let faces: Vec<Face> = document.shown(2).iter().map(|(_, face)| *face).collect();
        assert_eq!(
            faces,
            [
                Face::Link,
                Face::Strike,
                Face::Bold,
                Face::Italic,
                Face::Plain
            ]
        );
    }

    #[test]
    fn line_breaks_leave_the_text_and_control_characters_leave_the_screen() {
        let blocks = [
            paragraph("a\nb\r\nc\td\u{7}"),
            Block::Table(vec![strings(&["x\ny", "p\tq"])]),
        ];
        let document = text_document(&blocks);
        assert_eq!(
            document.lines(),
            ["a b  c\td\u{7}", "x y\tp q"],
            "a tab inside a cell would read as a cell boundary"
        );
        assert_eq!(document.shown_text(0), "a b  c d ");
        assert_eq!(document.shown_text(1), "│ x y │ p q │");
    }

    #[test]
    fn an_empty_document_has_no_lines_and_an_empty_paragraph_is_an_empty_line() {
        assert!(text_document(&[]).is_empty());
        let document = text_document(&[
            Block::Paragraph(Paragraph::default()),
            Block::Table(Vec::new()),
        ]);
        assert_eq!(document.lines(), [""]);
        assert!(document.shown(0).is_empty());
    }

    fn temp_docx(body: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("docrev-docx-source-{}.docx", uuid::Uuid::new_v4()));
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        zip.start_file(
            "word/document.xml",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(format!("<w:document><w:body>{body}</w:body></w:document>").as_bytes())
            .unwrap();
        zip.finish().unwrap();
        path
    }

    #[test]
    fn loads_a_docx_as_a_text_document_with_a_revision() {
        let path = temp_docx(
            "<w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr><w:r><w:t>T</w:t></w:r></w:p><w:p/>",
        );
        let document = DocxSource.load(&path).unwrap();
        let text = document.text().unwrap();
        assert_eq!(text.lines(), ["T", ""]);
        assert_eq!(text.shown(0), [("T".to_string(), Face::Heading(1))]);
        assert_eq!(DocxSource.kind(&path), DocumentKind::Text);
        assert!(DocxSource.revision(&path).is_some_and(|r| r != 0));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_missing_or_broken_file_is_an_open_error_naming_the_file() {
        let missing = std::env::temp_dir().join("docrev-docx-source-missing.docx");
        let err = DocxSource.load(&missing).unwrap_err();
        assert!(matches!(err, LoadError::Open(_)), "{err:?}");
        let message = err.to_string();
        assert!(
            message.starts_with("cannot open ") && message.contains("missing.docx"),
            "{message}"
        );
        assert_eq!(DocxSource.revision(&missing), Some(0));

        let broken =
            std::env::temp_dir().join(format!("docrev-docx-source-{}.docx", uuid::Uuid::new_v4()));
        std::fs::write(&broken, b"not a zip").unwrap();
        let err = DocxSource.load(&broken).unwrap_err();
        assert!(err.to_string().contains("invalid Zip archive"), "{err}");
        let _ = std::fs::remove_file(broken);
    }
}
