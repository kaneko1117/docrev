//! The body of a Word document, read straight from the archive.

use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use thiserror::Error;

use super::ooxml::{
    attr_value, cdata_piece, open_archive, read_entry, reference_piece, text_piece,
};

#[derive(Debug, Error)]
#[error("{0}")]
pub struct DocxError(String);

/// How a run is set in the file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Emphasis {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub link: bool,
}

/// (text, how it is set); neighbours never share an emphasis.
pub type Span = (String, Emphasis);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Paragraph {
    /// 1 to 6 for a paragraph styled `heading N` or `Title`.
    pub heading: Option<u8>,
    /// The level of a numbered or bulleted paragraph, 0 for the outermost.
    pub list_level: Option<u8>,
    pub spans: Vec<Span>,
}

impl Paragraph {
    pub fn text(&self) -> String {
        self.spans.iter().map(|(text, _)| text.as_str()).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Paragraph(Paragraph),
    /// Rows of cell text in reading order; a cell's paragraphs are joined by a space.
    Table(Vec<Vec<String>>),
}

/// The body in document order, tracked changes accepted; headers, footers, footnotes, drawings
/// and text boxes are left out.
pub fn read_blocks(document: &Path) -> Result<Vec<Block>, DocxError> {
    let mut archive = open_archive(document).map_err(DocxError)?;
    let part = main_part(&mut archive);
    let xml = read_entry(&mut archive, &part).map_err(DocxError)?;
    let styles = read_entry(&mut archive, &styles_part(&part))
        .ok()
        .and_then(|xml| parse_style_names(&xml).ok())
        .unwrap_or_default();
    parse_body(&xml, &styles)
}

/// The part the package relationships name as the document, else `word/document.xml`.
fn main_part(archive: &mut zip::ZipArchive<File>) -> String {
    read_entry(archive, "_rels/.rels")
        .ok()
        .and_then(|xml| office_document_target(&xml))
        .unwrap_or_else(|| "word/document.xml".to_string())
}

fn office_document_target(xml: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event().ok()? {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"Relationship" => {
                let kind = attribute(&e, b"Type", reader.decoder()).unwrap_or_default();
                let target = attribute(&e, b"Target", reader.decoder()).unwrap_or_default();
                if kind.ends_with("/officeDocument") && !target.is_empty() {
                    let target = target.trim_start_matches('/');
                    return Some(target.strip_prefix("./").unwrap_or(target).to_string());
                }
            }
            Event::Eof => return None,
            _ => {}
        }
    }
}

/// `styles.xml` next to the main part.
fn styles_part(main: &str) -> String {
    match main.rsplit_once('/') {
        Some((dir, _)) => format!("{dir}/styles.xml"),
        None => "styles.xml".to_string(),
    }
}

/// `w:styleId` → `w:name` lowercased; Word keeps the English name whatever the UI language.
pub(crate) fn parse_style_names(xml: &str) -> Result<HashMap<String, String>, DocxError> {
    let mut reader = Reader::from_str(xml);
    let mut names = HashMap::new();
    let mut current: Option<String> = None;
    loop {
        match reader.read_event().map_err(|e| DocxError(e.to_string()))? {
            Event::Start(e) if e.local_name().as_ref() == b"style" => {
                current = attribute(&e, b"styleId", reader.decoder());
            }
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"name" => {
                if let (Some(id), Some(name)) = (&current, attribute(&e, b"val", reader.decoder()))
                {
                    names.insert(id.clone(), name.to_lowercase());
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"style" => current = None,
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(names)
}

fn attribute(element: &BytesStart, name: &[u8], decoder: quick_xml::Decoder) -> Option<String> {
    element
        .attributes()
        .flatten()
        .find(|attr| attr.key.local_name().as_ref() == name)
        .map(|attr| attr_value(&attr, decoder))
}

/// `heading 3` → 3 (capped at 6), `title` → 1; the style id is accepted in place of the name.
fn heading_level(style: &str) -> Option<u8> {
    if style == "title" {
        return Some(1);
    }
    let level: u8 = style.strip_prefix("heading")?.trim().parse().ok()?;
    (1..=9).contains(&level).then_some(level.min(6))
}

pub(crate) fn parse_body(
    xml: &str,
    styles: &HashMap<String, String>,
) -> Result<Vec<Block>, DocxError> {
    let mut reader = Reader::from_str(xml);
    let mut body = Body::new(styles);
    loop {
        match reader.read_event().map_err(|e| DocxError(e.to_string()))? {
            Event::Start(e) => body.start(&e, reader.decoder()),
            Event::Empty(e) => body.empty(&e, reader.decoder()),
            Event::End(e) => body.end(e.local_name().as_ref()),
            Event::Text(t) => body.text(&text_piece(&t).map_err(DocxError)?),
            Event::CData(c) => body.text(&cdata_piece(&c).map_err(DocxError)?),
            Event::GeneralRef(r) => body.text(&reference_piece(&r).map_err(DocxError)?),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(body.blocks)
}

/// Subtrees that hold no body text, or text a reader does not see; only the `Choice` branch of an
/// `mc:AlternateContent` is read, never its `Fallback` too.
const SKIPPED: [&[u8]; 13] = [
    b"del",
    b"moveFrom",
    b"drawing",
    b"pict",
    b"object",
    b"txbxContent",
    b"rt",
    b"sdtPr",
    b"sdtEndPr",
    b"sectPr",
    b"pPrChange",
    b"rPrChange",
    b"Fallback",
];

struct Body<'a> {
    styles: &'a HashMap<String, String>,
    blocks: Vec<Block>,
    /// Open tables, innermost last; a paragraph inside one goes to its last cell.
    tables: Vec<Vec<Vec<String>>>,
    paragraph: Option<Paragraph>,
    /// Open runs, innermost last; a run inside another (ruby) flushes the outer text first.
    runs: Vec<Span>,
    depth: usize,
    /// The depth of the skipped subtree's root while inside one.
    skip_from: Option<usize>,
    in_paragraph_props: bool,
    in_text: bool,
    hyperlinks: usize,
}

impl<'a> Body<'a> {
    fn new(styles: &'a HashMap<String, String>) -> Self {
        Self {
            styles,
            blocks: Vec::new(),
            tables: Vec::new(),
            paragraph: None,
            runs: Vec::new(),
            depth: 0,
            skip_from: None,
            in_paragraph_props: false,
            in_text: false,
            hyperlinks: 0,
        }
    }

    fn start(&mut self, element: &BytesStart, decoder: quick_xml::Decoder) {
        let name = element.local_name();
        let name = name.as_ref();
        if self.skip_from.is_none() {
            if SKIPPED.contains(&name) {
                self.skip_from = Some(self.depth);
            } else {
                self.open(name, attribute(element, b"val", decoder));
            }
        }
        self.depth += 1;
    }

    fn empty(&mut self, element: &BytesStart, decoder: quick_xml::Decoder) {
        let name = element.local_name();
        let name = name.as_ref();
        if self.skip_from.is_some() || SKIPPED.contains(&name) {
            return;
        }
        self.open(name, attribute(element, b"val", decoder));
        self.close(name);
    }

    fn end(&mut self, name: &[u8]) {
        self.depth = self.depth.saturating_sub(1);
        if self.skip_from == Some(self.depth) {
            self.skip_from = None;
        } else if self.skip_from.is_none() {
            self.close(name);
        }
    }

    fn text(&mut self, piece: &str) {
        if self.skip_from.is_none()
            && self.in_text
            && let Some((text, _)) = self.runs.last_mut()
        {
            text.push_str(piece);
        }
    }

    fn open(&mut self, name: &[u8], val: Option<String>) {
        match name {
            b"p" => self.paragraph = Some(Paragraph::default()),
            b"pPr" => self.in_paragraph_props = true,
            b"r" => {
                if let Some((text, emphasis)) = self.runs.last_mut() {
                    let outer = (std::mem::take(text), *emphasis);
                    self.flush(outer);
                }
                let emphasis = Emphasis {
                    link: self.hyperlinks > 0,
                    ..Emphasis::default()
                };
                self.runs.push((String::new(), emphasis));
            }
            b"t" => self.in_text = !self.runs.is_empty(),
            b"hyperlink" => self.hyperlinks += 1,
            b"tbl" => self.tables.push(Vec::new()),
            b"tr" => {
                if let Some(table) = self.tables.last_mut() {
                    table.push(Vec::new());
                }
            }
            b"tc" => {
                if let Some(row) = self.tables.last_mut().and_then(|t| t.last_mut()) {
                    row.push(String::new());
                }
            }
            _ if self.in_paragraph_props => self.paragraph_property(name, val),
            _ => self.run_content(name, val),
        }
    }

    fn close(&mut self, name: &[u8]) {
        match name {
            b"p" => self.end_paragraph(),
            b"pPr" => self.in_paragraph_props = false,
            b"r" => self.end_run(),
            b"t" => self.in_text = false,
            b"hyperlink" => self.hyperlinks = self.hyperlinks.saturating_sub(1),
            b"tbl" => self.end_table(),
            _ => {}
        }
    }

    fn paragraph_property(&mut self, name: &[u8], val: Option<String>) {
        let Some(paragraph) = &mut self.paragraph else {
            return;
        };
        match name {
            b"pStyle" => {
                let id = val.unwrap_or_default();
                let style = self
                    .styles
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| id.to_lowercase());
                paragraph.heading = heading_level(&style);
            }
            b"numPr" => paragraph.list_level = Some(0),
            b"ilvl" => paragraph.list_level = val.and_then(|v| v.parse().ok()).or(Some(0)),
            // numId 0 removes the numbering a style would give
            b"numId" if val.as_deref() == Some("0") => paragraph.list_level = None,
            _ => {}
        }
    }

    fn run_content(&mut self, name: &[u8], val: Option<String>) {
        let Some((text, emphasis)) = self.runs.last_mut() else {
            return;
        };
        match name {
            b"tab" | b"ptab" => text.push('\t'),
            b"br" | b"cr" => text.push(' '),
            b"noBreakHyphen" => text.push('-'),
            b"b" => emphasis.bold = is_on(val.as_deref()),
            b"i" => emphasis.italic = is_on(val.as_deref()),
            b"strike" | b"dstrike" => emphasis.strike = is_on(val.as_deref()),
            _ => {}
        }
    }

    fn end_run(&mut self) {
        if let Some(run) = self.runs.pop() {
            self.flush(run);
        }
    }

    /// Adds the run's text to the paragraph, joining it to an alike neighbour.
    fn flush(&mut self, (text, emphasis): Span) {
        if text.is_empty() {
            return;
        }
        let Some(paragraph) = &mut self.paragraph else {
            return;
        };
        match paragraph.spans.last_mut() {
            Some((last, e)) if *e == emphasis => last.push_str(&text),
            _ => paragraph.spans.push((text, emphasis)),
        }
    }

    fn end_paragraph(&mut self) {
        let Some(paragraph) = self.paragraph.take() else {
            return;
        };
        match self.current_cell() {
            Some(cell) => append_words(cell, &paragraph.text()),
            None => self.blocks.push(Block::Paragraph(paragraph)),
        }
    }

    /// A nested table's text flattens into the cell that holds it.
    fn end_table(&mut self) {
        let Some(rows) = self.tables.pop() else {
            return;
        };
        match self.current_cell() {
            Some(cell) => {
                for text in rows.iter().flatten() {
                    append_words(cell, text);
                }
            }
            None => self.blocks.push(Block::Table(rows)),
        }
    }

    fn current_cell(&mut self) -> Option<&mut String> {
        self.tables.last_mut()?.last_mut()?.last_mut()
    }
}

/// `<w:b/>` is on; `<w:b w:val="0"/>` is off.
fn is_on(val: Option<&str>) -> bool {
    !matches!(val, Some("0" | "false" | "off"))
}

fn append_words(cell: &mut String, text: &str) {
    if text.is_empty() {
        return;
    }
    if !cell.is_empty() {
        cell.push(' ');
    }
    cell.push_str(text);
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn document(body: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}<w:sectPr><w:pgSz w:w="11906"/></w:sectPr></w:body></w:document>"#
        )
    }

    fn blocks(body: &str) -> Vec<Block> {
        parse_body(&document(body), &HashMap::new()).unwrap()
    }

    fn paragraph(runs: &str) -> String {
        format!("<w:p>{runs}</w:p>")
    }

    fn run(text: &str) -> String {
        format!("<w:r><w:t xml:space=\"preserve\">{text}</w:t></w:r>")
    }

    fn texts(blocks: &[Block]) -> Vec<String> {
        blocks
            .iter()
            .map(|block| match block {
                Block::Paragraph(p) => p.text(),
                Block::Table(rows) => rows
                    .iter()
                    .map(|row| row.join("|"))
                    .collect::<Vec<_>>()
                    .join("/"),
            })
            .collect()
    }

    fn only_paragraph(blocks: Vec<Block>) -> Paragraph {
        match blocks.as_slice() {
            [Block::Paragraph(p)] => p.clone(),
            other => panic!("expected one paragraph, got {other:?}"),
        }
    }

    #[test]
    fn runs_keep_their_text_and_emphasis_and_alike_neighbours_merge() {
        let p = only_paragraph(blocks(&paragraph(
            r#"<w:r><w:t xml:space="preserve">Hello </w:t></w:r><w:r><w:t>wor</w:t></w:r><w:r><w:rPr><w:sz w:val="24"/></w:rPr><w:t>ld</w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>!</w:t></w:r><w:r><w:rPr><w:b w:val="0"/><w:i/><w:strike/></w:rPr><w:t>?</w:t></w:r><w:r><w:rPr><w:b w:val="true"/></w:rPr><w:t/></w:r>"#,
        )));
        let bold = Emphasis {
            bold: true,
            ..Emphasis::default()
        };
        let italic_strike = Emphasis {
            italic: true,
            strike: true,
            ..Emphasis::default()
        };
        assert_eq!(
            p.spans,
            [
                ("Hello world".to_string(), Emphasis::default()),
                ("!".to_string(), bold),
                ("?".to_string(), italic_strike),
            ]
        );
        assert_eq!(p.text(), "Hello world!?");
        assert_eq!((p.heading, p.list_level), (None, None));
    }

    #[test]
    fn empty_paragraphs_are_kept_so_numbers_agree_with_word() {
        let blocks = blocks(&format!(
            "<w:p/>{}<w:p><w:pPr><w:jc w:val=\"center\"/></w:pPr></w:p>{}",
            paragraph(&run("a")),
            paragraph("<w:r><w:t></w:t></w:r>")
        ));
        assert_eq!(texts(&blocks), ["", "a", "", ""]);
    }

    #[test]
    fn tabs_breaks_hyphens_entities_and_preserved_spaces_become_text() {
        let p = only_paragraph(blocks(&paragraph(
            r#"<w:r><w:t>a</w:t><w:tab/><w:t>b</w:t><w:br/><w:t>c</w:t><w:cr/><w:t>d</w:t><w:noBreakHyphen/><w:t>e</w:t><w:softHyphen/><w:t>f</w:t><w:br w:type="page"/><w:t xml:space="preserve"> &amp;&#x3042;&lt; </w:t></w:r>"#,
        )));
        assert_eq!(p.text(), "a\tb c d-ef  &あ< ");
    }

    #[test]
    fn a_tab_stop_in_the_paragraph_properties_is_not_a_tab() {
        let p = only_paragraph(blocks(&paragraph(&format!(
            r#"<w:pPr><w:tabs><w:tab w:val="left" w:pos="720"/></w:tabs><w:rPr><w:b/></w:rPr></w:pPr>{}"#,
            run("x")
        ))));
        assert_eq!(p.spans, [("x".to_string(), Emphasis::default())]);
    }

    #[test]
    fn headings_come_from_the_style_name_or_failing_that_the_style_id() {
        let styles: HashMap<String, String> = [
            ("1", "heading 1"),
            ("Heading2", "heading 2"),
            ("Title", "title"),
            ("a", "normal"),
            ("Deep", "heading 9"),
        ]
        .into_iter()
        .map(|(id, name)| (id.to_string(), name.to_string()))
        .collect();
        let body: String = [
            "1",
            "Heading2",
            "Title",
            "a",
            "Deep",
            "Heading3",
            "Heading10",
        ]
        .iter()
        .map(|id| {
            paragraph(&format!(
                "<w:pPr><w:pStyle w:val=\"{id}\"/></w:pPr>{}",
                run("t")
            ))
        })
        .collect();
        let blocks = parse_body(&document(&body), &styles).unwrap();
        let headings: Vec<Option<u8>> = blocks
            .iter()
            .map(|b| match b {
                Block::Paragraph(p) => p.heading,
                Block::Table(_) => unreachable!(),
            })
            .collect();
        assert_eq!(
            headings,
            [Some(1), Some(2), Some(1), None, Some(6), Some(3), None]
        );
    }

    #[test]
    fn list_paragraphs_carry_their_level_and_num_id_zero_removes_it() {
        let body = [
            r#"<w:pPr><w:numPr><w:ilvl w:val="2"/><w:numId w:val="5"/></w:numPr></w:pPr>"#,
            r#"<w:pPr><w:numPr><w:numId w:val="5"/></w:numPr></w:pPr>"#,
            r#"<w:pPr><w:numPr><w:ilvl w:val="1"/><w:numId w:val="0"/></w:numPr></w:pPr>"#,
            "",
        ]
        .iter()
        .map(|props| paragraph(&format!("{props}{}", run("t"))))
        .collect::<String>();
        let levels: Vec<Option<u8>> = blocks(&body)
            .iter()
            .map(|b| match b {
                Block::Paragraph(p) => p.list_level,
                Block::Table(_) => unreachable!(),
            })
            .collect();
        assert_eq!(levels, [Some(2), Some(0), None, None]);
    }

    #[test]
    fn tables_become_rows_of_cell_text_and_nested_tables_flatten_into_their_cell() {
        let cell = |inner: &str| format!("<w:tc><w:tcPr><w:tcW w:w=\"1\"/></w:tcPr>{inner}</w:tc>");
        let table = format!(
            "<w:tbl><w:tblPr><w:tblStyle w:val=\"x\"/></w:tblPr><w:tr><w:trPr/>{}{}</w:tr><w:tr>{}{}</w:tr></w:tbl>",
            cell(&format!("{}{}", paragraph(&run("a")), paragraph(&run("b")))),
            cell(&paragraph(&run("c"))),
            cell("<w:p/>"),
            cell(&paragraph(&run("d"))),
        );
        let parsed = blocks(&format!("{table}{}", paragraph(&run("after"))));
        assert_eq!(
            parsed,
            [
                Block::Table(vec![
                    vec!["a b".to_string(), "c".to_string()],
                    vec![String::new(), "d".to_string()],
                ]),
                Block::Paragraph(Paragraph {
                    spans: vec![("after".to_string(), Emphasis::default())],
                    ..Paragraph::default()
                }),
            ]
        );

        let nested = format!(
            "<w:tbl><w:tr>{}</w:tr></w:tbl>",
            cell(&format!(
                "{}<w:tbl><w:tr>{}{}</w:tr></w:tbl>{}",
                paragraph(&run("outer")),
                cell(&paragraph(&run("in1"))),
                cell(&paragraph(&run("in2"))),
                paragraph(&run("end"))
            ))
        );
        assert_eq!(texts(&blocks(&nested)), ["outer in1 in2 end"]);
    }

    #[test]
    fn tracked_changes_read_as_accepted_and_old_formatting_is_ignored() {
        let p = only_paragraph(blocks(&paragraph(&format!(
            r#"{}<w:ins w:id="1"><w:r><w:t xml:space="preserve">new </w:t></w:r></w:ins><w:del w:id="2"><w:r><w:delText xml:space="preserve">old </w:delText></w:r></w:del><w:moveFrom w:id="3"><w:r><w:t>moved</w:t></w:r></w:moveFrom><w:moveTo w:id="4"><w:r><w:t>here</w:t></w:r></w:moveTo><w:r><w:rPr><w:rPrChange w:id="5"><w:rPr><w:b/></w:rPr></w:rPrChange></w:rPr><w:t> plain</w:t></w:r>"#,
            run("keep ")
        ))));
        assert_eq!(
            p.spans,
            [("keep new here plain".to_string(), Emphasis::default())]
        );
    }

    #[test]
    fn content_controls_hyperlinks_and_fields_show_their_text() {
        let body = format!(
            r#"<w:sdt><w:sdtPr><w:alias w:val="x"/><w:rPr><w:b/></w:rPr></w:sdtPr><w:sdtContent>{}</w:sdtContent></w:sdt><w:p><w:hyperlink r:id="rId1"><w:r><w:rPr><w:rStyle w:val="Hyperlink"/></w:rPr><w:t>link</w:t></w:r></w:hyperlink>{}<w:fldSimple w:instr="PAGE"><w:r><w:t>3</w:t></w:r></w:fldSimple><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>DATE</w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>2026</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#,
            paragraph(&run("inside")),
            run(" after ")
        );
        let blocks = blocks(&body);
        assert_eq!(texts(&blocks), ["inside", "link after 32026"]);
        let Block::Paragraph(second) = &blocks[1] else {
            unreachable!()
        };
        let link = Emphasis {
            link: true,
            ..Emphasis::default()
        };
        assert_eq!(
            second.spans,
            [
                ("link".to_string(), link),
                (" after 32026".to_string(), Emphasis::default()),
            ]
        );
        let Block::Paragraph(first) = &blocks[0] else {
            unreachable!()
        };
        assert_eq!(first.spans, [("inside".to_string(), Emphasis::default())]);
    }

    #[test]
    fn only_the_choice_branch_of_alternate_content_is_read() {
        let p = only_paragraph(blocks(&paragraph(&format!(
            r#"<mc:AlternateContent><mc:Choice Requires="w14">{}</mc:Choice><mc:Fallback>{}</mc:Fallback></mc:AlternateContent>"#,
            run("A"),
            run("A")
        ))));
        assert_eq!(p.text(), "A");
    }

    #[test]
    fn cdata_is_text() {
        let p = only_paragraph(blocks(&paragraph(
            "<w:r><w:t><![CDATA[hello & <world>]]></w:t></w:r>",
        )));
        assert_eq!(p.text(), "hello & <world>");
    }

    #[test]
    fn a_ruby_inside_a_run_keeps_the_text_around_it_in_order() {
        let p = only_paragraph(blocks(&paragraph(
            r#"<w:r><w:rPr><w:b/></w:rPr><w:t>pre</w:t><w:ruby><w:rt><w:r><w:t>かん</w:t></w:r></w:rt><w:rubyBase><w:r><w:t>漢</w:t></w:r></w:rubyBase></w:ruby><w:t>post</w:t></w:r>"#,
        )));
        let bold = Emphasis {
            bold: true,
            ..Emphasis::default()
        };
        assert_eq!(
            p.spans,
            [
                ("pre".to_string(), bold),
                ("漢".to_string(), Emphasis::default()),
                ("post".to_string(), bold),
            ]
        );
    }

    #[test]
    fn drawings_text_boxes_and_ruby_readings_are_left_out() {
        let boxed = paragraph(&run("boxed"));
        let body = format!(
            r#"<w:p>{}<w:r><mc:AlternateContent><mc:Choice Requires="wps"><w:drawing><wp:anchor><a:graphic><wps:txbx><w:txbxContent>{boxed}</w:txbxContent></wps:txbx></a:graphic></wp:anchor></w:drawing></mc:Choice><mc:Fallback><w:pict><v:shape><v:textbox><w:txbxContent>{boxed}</w:txbxContent></v:textbox></v:shape></w:pict></mc:Fallback></mc:AlternateContent></w:r><w:r><w:ruby><w:rubyPr><w:rubyAlign w:val="center"/></w:rubyPr><w:rt><w:r><w:t>かん</w:t></w:r></w:rt><w:rubyBase><w:r><w:t>漢</w:t></w:r></w:rubyBase></w:ruby></w:r>{}<w:r><w:object><v:shape/></w:object></w:r></w:p>"#,
            run("text "),
            run("字")
        );
        assert_eq!(texts(&blocks(&body)), ["text 漢字"]);
    }

    #[test]
    fn style_names_are_read_by_id_and_lowercased() {
        let xml = r#"<w:styles><w:docDefaults/><w:latentStyles><w:lsdException w:name="Normal"/></w:latentStyles><w:style w:type="paragraph" w:styleId="1"><w:name w:val="heading 1"/><w:basedOn w:val="a"/></w:style><w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/></w:style><w:style w:type="character" w:styleId="Hyperlink"><w:name w:val="Hyperlink"/></w:style></w:styles>"#;
        let names = parse_style_names(xml).unwrap();
        assert_eq!(names.get("1").map(String::as_str), Some("heading 1"));
        assert_eq!(names.get("Title").map(String::as_str), Some("title"));
        assert_eq!(
            names.get("Hyperlink").map(String::as_str),
            Some("hyperlink")
        );
        assert_eq!(names.len(), 3);
    }

    #[test]
    fn malformed_xml_is_an_error_not_a_panic() {
        let err = parse_body("<w:document><w:body><w:p><w:r>", &HashMap::new());
        assert!(err.is_ok(), "an unclosed tree is tolerated");
        let err = parse_body("<w:document><w:body></w:p></w:document>", &HashMap::new());
        assert!(err.is_err(), "a mismatched end tag is a parse error");
    }

    /// A temporary file removed on drop, so a failed assertion leaves nothing behind.
    struct TempFile(std::path::PathBuf);

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn temp_path() -> TempFile {
        TempFile(std::env::temp_dir().join(format!("docrev-docx-{}.docx", uuid::Uuid::new_v4())))
    }

    fn archive(parts: &[(&str, &str)]) -> TempFile {
        let temp = temp_path();
        let file = File::create(&temp.0).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        for (name, content) in parts {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        temp
    }

    const STYLES: &str = r#"<w:styles><w:style w:type="paragraph" w:styleId="1"><w:name w:val="heading 1"/></w:style></w:styles>"#;

    #[test]
    fn an_archive_reads_its_document_part_and_resolves_headings_through_styles() {
        let body = format!(
            "{}{}",
            paragraph(&format!(
                "<w:pPr><w:pStyle w:val=\"1\"/></w:pPr>{}",
                run("Title")
            )),
            paragraph(&run("Body"))
        );
        let path = archive(&[
            ("word/document.xml", &document(&body)),
            ("word/styles.xml", STYLES),
        ]);
        let blocks = read_blocks(&path.0).unwrap();
        assert_eq!(texts(&blocks), ["Title", "Body"]);
        let Block::Paragraph(first) = &blocks[0] else {
            unreachable!()
        };
        assert_eq!(first.heading, Some(1));
    }

    #[test]
    fn the_package_relationships_name_the_document_part_and_styles_are_optional() {
        let rels = |target: &str| {
            format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="{target}"/></Relationships>"#
            )
        };
        let body = paragraph(&format!(
            "<w:pPr><w:pStyle w:val=\"Heading2\"/></w:pPr>{}",
            run("Two")
        ));
        for (target, part) in [
            ("/word/main.xml", "word/main.xml"),
            ("./word/main.xml", "word/main.xml"),
            ("doc/main.xml", "doc/main.xml"),
        ] {
            let path = archive(&[("_rels/.rels", &rels(target)), (part, &document(&body))]);
            let blocks = read_blocks(&path.0).unwrap();
            let Block::Paragraph(first) = &blocks[0] else {
                unreachable!()
            };
            assert_eq!(
                (first.text(), first.heading),
                ("Two".to_string(), Some(2)),
                "{target}"
            );
        }
    }

    #[test]
    fn a_broken_archive_is_a_typed_error() {
        let not_zip = temp_path();
        std::fs::write(&not_zip.0, b"this is not a zip file").unwrap();
        let err = read_blocks(&not_zip.0).unwrap_err();
        assert!(err.to_string().contains(".docx"), "{err}");

        let no_document = archive(&[("word/styles.xml", STYLES)]);
        let err = read_blocks(&no_document.0).unwrap_err();
        assert!(err.to_string().contains("word/document.xml"), "{err}");

        let malformed = archive(&[("word/document.xml", "<w:document><w:body></w:p>")]);
        assert!(read_blocks(&malformed.0).is_err());

        let missing = std::env::temp_dir().join("docrev-docx-missing.docx");
        assert!(read_blocks(&missing).is_err());
    }
}
