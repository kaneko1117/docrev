use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use quick_xml::Reader;
use quick_xml::events::Event;
use quick_xml::events::attributes::Attribute;
use thiserror::Error;

use crate::domain::anchor::Anchor;

#[derive(Debug, Error)]
#[error("{0}")]
pub struct MetaError(String);

/// 1-based column range with its Excel width (in characters).
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnWidth {
    pub min: u32,
    pub max: u32,
    pub width: f64,
}

/// Custom column widths per sheet name. calamine does not expose them, so
/// this reads `<cols>` from the sheet XML inside the xlsx zip directly.
pub fn column_widths(document: &Path) -> Result<HashMap<String, Vec<ColumnWidth>>, MetaError> {
    let mut archive = open_archive(document)?;

    let workbook = read_entry(&mut archive, "xl/workbook.xml")?;
    let rels = read_entry(&mut archive, "xl/_rels/workbook.xml.rels")?;
    let sheets = parse_sheet_ids(&workbook)?;
    let targets = parse_rel_targets(&rels)?;

    let mut result = HashMap::new();
    for (name, rid) in sheets {
        let Some(target) = targets.get(&rid) else {
            continue;
        };
        let xml = read_entry(&mut archive, &entry_path(target))?;
        let cols = parse_cols(&xml)?;
        if !cols.is_empty() {
            result.insert(name, cols);
        }
    }
    Ok(result)
}

/// Frozen panes per sheet name: `(rows, cols)` pinned while scrolling.
/// calamine does not expose `<pane>` either, so this reads each sheet's
/// `<sheetView><pane xSplit ySplit state="frozen"/>` from the zip.
pub fn frozen_panes(document: &Path) -> Result<HashMap<String, (usize, usize)>, MetaError> {
    let mut archive = open_archive(document)?;

    let workbook = read_entry(&mut archive, "xl/workbook.xml")?;
    let rels = read_entry(&mut archive, "xl/_rels/workbook.xml.rels")?;
    let sheets = parse_sheet_ids(&workbook)?;
    let targets = parse_rel_targets(&rels)?;

    let mut result = HashMap::new();
    for (name, rid) in sheets {
        let Some(target) = targets.get(&rid) else {
            continue;
        };
        let Ok(xml) = read_entry(&mut archive, &entry_path(target)) else {
            continue;
        };
        if let Some(frozen) = parse_pane(&xml)? {
            result.insert(name, frozen);
        }
    }
    Ok(result)
}

/// The first frozen `<pane>`. Non-frozen splits measure their offsets in
/// twips, not cells, so anything without a frozen state is ignored; frozen
/// splits carry whole cell counts in `xSplit`/`ySplit`.
fn parse_pane(xml: &str) -> Result<Option<(usize, usize)>, MetaError> {
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event().map_err(|e| MetaError(e.to_string()))? {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"pane" => {
                let mut x = 0f64;
                let mut y = 0f64;
                let mut frozen = false;
                for attr in e.attributes().flatten() {
                    let value = attr_value(&attr, reader.decoder());
                    match attr.key.as_ref() {
                        b"xSplit" => x = value.parse().unwrap_or(0.0),
                        b"ySplit" => y = value.parse().unwrap_or(0.0),
                        b"state" => frozen = value == "frozen" || value == "frozenSplit",
                        _ => {}
                    }
                }
                if frozen && (x > 0.0 || y > 0.0) && x.is_finite() && y.is_finite() {
                    return Ok(Some((y.max(0.0) as usize, x.max(0.0) as usize)));
                }
                return Ok(None);
            }
            // panes live in <sheetViews> at the top; stop before cell data
            Event::Start(e) if e.local_name().as_ref() == b"sheetData" => return Ok(None),
            Event::Eof => return Ok(None),
            _ => {}
        }
    }
}

/// One workbook comment as read from the file, 0-based cell coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct RawWorkbookComment {
    pub row: u32,
    pub col: u32,
    pub author: String,
    pub body: String,
    pub resolved: bool,
    pub replies: Vec<(String, String)>,
}

/// The workbook's own comments per sheet name: Excel's legacy notes
/// (`xl/comments*.xml`) and threaded comments (`xl/threadedComments/*.xml`),
/// resolved through each worksheet's relationship part. calamine exposes
/// neither. When a cell has a threaded comment, Excel also writes a legacy
/// fallback note on it — the fallback is dropped, the thread wins.
pub fn workbook_comments(
    document: &Path,
) -> Result<HashMap<String, Vec<RawWorkbookComment>>, MetaError> {
    let mut archive = open_archive(document)?;

    let workbook = read_entry(&mut archive, "xl/workbook.xml")?;
    let rels = read_entry(&mut archive, "xl/_rels/workbook.xml.rels")?;
    let sheets = parse_sheet_ids(&workbook)?;
    let targets = parse_rel_targets(&rels)?;
    // person GUID → display name, for threaded comment authors
    let persons = match read_entry(&mut archive, "xl/persons/person.xml") {
        Ok(xml) => parse_persons(&xml).unwrap_or_default(),
        Err(_) => HashMap::new(),
    };

    let mut result = HashMap::new();
    for (name, rid) in sheets {
        let Some(target) = targets.get(&rid) else {
            continue;
        };
        let sheet_path = entry_path(target);
        // one sheet's unreadable comments must not strip the others'
        let Some(sheet_rels) = read_sheet_rels(&mut archive, &sheet_path) else {
            continue;
        };
        let mut comments: Vec<RawWorkbookComment> = Vec::new();
        for target in related_targets(&sheet_rels, &sheet_path, "threadedComment") {
            if let Ok(xml) = read_entry(&mut archive, &target)
                && let Ok(mut threaded) = parse_threaded_comments(&xml, &persons)
            {
                comments.append(&mut threaded);
            }
        }
        let threaded_cells: std::collections::HashSet<(u32, u32)> =
            comments.iter().map(|c| (c.row, c.col)).collect();
        for target in related_targets(&sheet_rels, &sheet_path, "comments") {
            if let Ok(xml) = read_entry(&mut archive, &target)
                && let Ok(notes) = parse_legacy_comments(&xml)
            {
                comments.extend(
                    notes
                        .into_iter()
                        .filter(|n| !threaded_cells.contains(&(n.row, n.col))),
                );
            }
        }
        if !comments.is_empty() {
            comments.sort_by_key(|c| (c.row, c.col));
            result.insert(name, comments);
        }
    }
    Ok(result)
}

/// The rels part beside a worksheet: `worksheets/sheet1.xml` →
/// `worksheets/_rels/sheet1.xml.rels`. Absent for most sheets.
fn read_sheet_rels(archive: &mut zip::ZipArchive<File>, sheet_path: &str) -> Option<String> {
    let (dir, file) = sheet_path.rsplit_once('/')?;
    read_entry(archive, &format!("{dir}/_rels/{file}.rels")).ok()
}

/// Targets of relationships whose type ends with `/<kind>`, resolved
/// relative to the worksheet's directory (`../comments1.xml` → `xl/...`).
fn related_targets(rels_xml: &str, sheet_path: &str, kind: &str) -> Vec<String> {
    let mut reader = Reader::from_str(rels_xml);
    let mut out = Vec::new();
    let base = sheet_path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    loop {
        match reader.read_event() {
            Ok(Event::Start(e) | Event::Empty(e)) if e.local_name().as_ref() == b"Relationship" => {
                let mut ty = String::new();
                let mut target = String::new();
                for attr in e.attributes().flatten() {
                    match attr.key.as_ref() {
                        b"Type" => ty = attr_value(&attr, reader.decoder()),
                        b"Target" => target = attr_value(&attr, reader.decoder()),
                        _ => {}
                    }
                }
                if ty.ends_with(&format!("/{kind}")) && !target.is_empty() {
                    out.push(resolve_relative(base, &target));
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

/// `xl/worksheets` + `../comments1.xml` → `xl/comments1.xml`.
fn resolve_relative(base: &str, target: &str) -> String {
    if let Some(absolute) = target.strip_prefix('/') {
        return absolute.to_string();
    }
    let mut parts: Vec<&str> = base.split('/').filter(|p| !p.is_empty()).collect();
    for piece in target.split('/') {
        match piece {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            other => parts.push(other),
        }
    }
    parts.join("/")
}

/// `<person displayName="田中" id="{GUID}"/>` pairs.
fn parse_persons(xml: &str) -> Result<HashMap<String, String>, MetaError> {
    let mut reader = Reader::from_str(xml);
    let mut out = HashMap::new();
    loop {
        match reader.read_event().map_err(|e| MetaError(e.to_string()))? {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"person" => {
                let mut id = String::new();
                let mut name = String::new();
                for attr in e.attributes().flatten() {
                    match attr.key.as_ref() {
                        b"id" => id = attr_value(&attr, reader.decoder()),
                        b"displayName" => name = attr_value(&attr, reader.decoder()),
                        _ => {}
                    }
                }
                if !id.is_empty() {
                    out.insert(id, name);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

/// Legacy notes: `<authors>` by index, then `<comment ref author_id>` with
/// the body as the concatenation of its `<t>` runs.
fn parse_legacy_comments(xml: &str) -> Result<Vec<RawWorkbookComment>, MetaError> {
    let mut reader = Reader::from_str(xml);
    let mut authors: Vec<String> = Vec::new();
    let mut out = Vec::new();
    let mut in_author = false;
    let mut current: Option<RawWorkbookComment> = None;
    let mut in_text = false;
    loop {
        match reader.read_event().map_err(|e| MetaError(e.to_string()))? {
            Event::Start(e) => match e.local_name().as_ref() {
                b"author" => in_author = true,
                b"comment" => {
                    let mut cell = None;
                    let mut author_id = None;
                    for attr in e.attributes().flatten() {
                        match attr.key.as_ref() {
                            b"ref" => {
                                cell = crate::domain::anchor::Anchor::parse_cell_ref(&attr_value(
                                    &attr,
                                    reader.decoder(),
                                ))
                            }
                            b"authorId" => {
                                author_id =
                                    attr_value(&attr, reader.decoder()).parse::<usize>().ok()
                            }
                            _ => {}
                        }
                    }
                    if let Some((row, col)) = cell {
                        current = Some(RawWorkbookComment {
                            row,
                            col,
                            author: author_id
                                .and_then(|i| authors.get(i))
                                .cloned()
                                .unwrap_or_default(),
                            body: String::new(),
                            resolved: false,
                            replies: Vec::new(),
                        });
                    }
                }
                b"t" if current.is_some() => in_text = true,
                _ => {}
            },
            Event::Text(t) => {
                let text = t
                    .xml_content(quick_xml::XmlVersion::Implicit1_0)
                    .map_err(|e| MetaError(e.to_string()))?;
                if in_author {
                    authors.push(text.into_owned());
                    in_author = false;
                } else if in_text && let Some(comment) = &mut current {
                    comment.body.push_str(&text);
                }
            }
            Event::End(e) => match e.local_name().as_ref() {
                b"t" => in_text = false,
                b"author" => in_author = false,
                b"comment" => {
                    if let Some(comment) = current.take()
                        && !comment.body.trim().is_empty()
                    {
                        out.push(comment);
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

/// Threaded comments: roots have no `parentId`; replies attach to their
/// parent in file order. `done="1"` on the root maps to resolved.
fn parse_threaded_comments(
    xml: &str,
    persons: &HashMap<String, String>,
) -> Result<Vec<RawWorkbookComment>, MetaError> {
    struct Entry {
        id: String,
        comment: RawWorkbookComment,
    }
    let mut reader = Reader::from_str(xml);
    let mut roots: Vec<Entry> = Vec::new();
    // (parent id, author, body) until all roots are known
    let mut replies: Vec<(String, String, String)> = Vec::new();
    let mut in_text = false;
    // (is_reply, parent, buffer target index into roots or replies)
    let mut open: Option<bool> = None;
    loop {
        match reader.read_event().map_err(|e| MetaError(e.to_string()))? {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"threadedComment" => {
                let mut cell = None;
                let mut id = String::new();
                let mut parent = None;
                let mut person = String::new();
                let mut done = false;
                for attr in e.attributes().flatten() {
                    let value = attr_value(&attr, reader.decoder());
                    match attr.key.as_ref() {
                        b"ref" => cell = crate::domain::anchor::Anchor::parse_cell_ref(&value),
                        b"id" => id = value,
                        b"parentId" => parent = Some(value),
                        b"personId" => person = value,
                        b"done" => done = value == "1" || value == "true",
                        _ => {}
                    }
                }
                let author = persons.get(&person).cloned().unwrap_or_default();
                match (parent, cell) {
                    (Some(parent), _) => {
                        replies.push((parent, author, String::new()));
                        open = Some(true);
                    }
                    (None, Some((row, col))) => {
                        roots.push(Entry {
                            id,
                            comment: RawWorkbookComment {
                                row,
                                col,
                                author,
                                body: String::new(),
                                resolved: done,
                                replies: Vec::new(),
                            },
                        });
                        open = Some(false);
                    }
                    _ => open = None,
                }
            }
            Event::Start(e) if e.local_name().as_ref() == b"text" => in_text = open.is_some(),
            Event::Text(t) if in_text => {
                let text = t
                    .xml_content(quick_xml::XmlVersion::Implicit1_0)
                    .map_err(|e| MetaError(e.to_string()))?;
                match open {
                    Some(true) => {
                        if let Some((_, _, body)) = replies.last_mut() {
                            body.push_str(&text);
                        }
                    }
                    Some(false) => {
                        if let Some(entry) = roots.last_mut() {
                            entry.comment.body.push_str(&text);
                        }
                    }
                    None => {}
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"text" => in_text = false,
            Event::Eof => break,
            _ => {}
        }
    }
    for (parent, author, body) in replies {
        if let Some(entry) = roots.iter_mut().find(|r| r.id == parent) {
            entry.comment.replies.push((author, body));
        }
    }
    Ok(roots.into_iter().map(|e| e.comment).collect())
}

/// What a cell's `s=` style index resolves to: a number-format code, a
/// solid fill color and/or a font color (sRGB).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct CellStyle {
    pub format: Option<String>,
    pub fill: Option<(u8, u8, u8)>,
    pub font: Option<(u8, u8, u8)>,
}

impl CellStyle {
    fn is_plain(&self) -> bool {
        self.format.is_none() && self.fill.is_none() && self.font.is_none()
    }
}

/// Everything cells reference in styles.xml, plus which cells reference it:
/// `sheets` maps 0-based (row, col) positions to indices in `styles`.
#[derive(Debug, Default, Clone)]
pub struct WorkbookStyles {
    pub styles: Vec<CellStyle>,
    pub sheets: HashMap<String, HashMap<(u32, u32), usize>>,
}

/// Per-cell number formats and fill colors, resolved through styles.xml
/// (`numFmts` + `fills` + `cellXfs`) and each sheet's `s=` style indices.
/// calamine does not expose styles either. Theme-indexed fills resolve via
/// the workbook's theme1.xml, falling back to the default Office palette.
pub fn cell_styles(document: &Path) -> Result<WorkbookStyles, MetaError> {
    let mut archive = open_archive(document)?;

    // a workbook without a theme part legitimately resolves against the
    // default Office palette; a theme that exists but cannot be parsed must
    // not silently recolor cells, so its theme-indexed fills are dropped
    // (empty palette) while rgb fills stay untouched
    let palette = match read_entry(&mut archive, "xl/theme/theme1.xml") {
        Ok(xml) => parse_theme_palette(&xml).unwrap_or_default(),
        Err(_) => default_palette(),
    };
    let styles_xml = read_entry(&mut archive, "xl/styles.xml")?;
    let styles = parse_styles(&styles_xml, &palette)?;
    if styles.iter().all(CellStyle::is_plain) {
        return Ok(WorkbookStyles::default());
    }

    let workbook = read_entry(&mut archive, "xl/workbook.xml")?;
    let rels = read_entry(&mut archive, "xl/_rels/workbook.xml.rels")?;
    let sheets = parse_sheet_ids(&workbook)?;
    let targets = parse_rel_targets(&rels)?;

    let mut result = HashMap::new();
    for (name, rid) in sheets {
        let Some(target) = targets.get(&rid) else {
            continue;
        };
        // one unreadable sheet must not strip styles from the others
        let Ok(xml) = read_entry(&mut archive, &entry_path(target)) else {
            continue;
        };
        let Ok(cells) = parse_cell_styles(&xml, &styles) else {
            continue;
        };
        if !cells.is_empty() {
            result.insert(name, cells);
        }
    }
    Ok(WorkbookStyles {
        styles,
        sheets: result,
    })
}

fn open_archive(document: &Path) -> Result<zip::ZipArchive<File>, MetaError> {
    let file =
        File::open(document).map_err(|e| MetaError(format!("{}: {e}", document.display())))?;
    zip::ZipArchive::new(file).map_err(|e| MetaError(e.to_string()))
}

/// Relationship targets are zip paths relative to `xl/`, or absolute with `/`.
fn entry_path(target: &str) -> String {
    match target.strip_prefix('/') {
        Some(absolute) => absolute.to_string(),
        None => format!("xl/{target}"),
    }
}

fn read_entry(archive: &mut zip::ZipArchive<File>, name: &str) -> Result<String, MetaError> {
    let mut entry = archive
        .by_name(name)
        .map_err(|e| MetaError(format!("{name}: {e}")))?;
    let mut text = String::new();
    entry
        .read_to_string(&mut text)
        .map_err(|e| MetaError(format!("{name}: {e}")))?;
    Ok(text)
}

fn attr_value(attr: &Attribute, decoder: quick_xml::Decoder) -> String {
    attr.decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, decoder)
        .map(|v| v.into_owned())
        .unwrap_or_default()
}

/// `<sheet name="売上" r:id="rId1"/>` pairs from workbook.xml.
fn parse_sheet_ids(xml: &str) -> Result<Vec<(String, String)>, MetaError> {
    let mut reader = Reader::from_str(xml);
    let mut out = Vec::new();
    loop {
        match reader.read_event().map_err(|e| MetaError(e.to_string()))? {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"sheet" => {
                let mut name = None;
                let mut rid = None;
                for attr in e.attributes().flatten() {
                    match attr.key.as_ref() {
                        b"name" => name = Some(attr_value(&attr, reader.decoder())),
                        b"r:id" => rid = Some(attr_value(&attr, reader.decoder())),
                        _ => {}
                    }
                }
                if let (Some(name), Some(rid)) = (name, rid) {
                    out.push((name, rid));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

/// `<Relationship Id="rId1" Target="worksheets/sheet1.xml"/>` from the rels file.
fn parse_rel_targets(xml: &str) -> Result<HashMap<String, String>, MetaError> {
    let mut reader = Reader::from_str(xml);
    let mut out = HashMap::new();
    loop {
        match reader.read_event().map_err(|e| MetaError(e.to_string()))? {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"Relationship" => {
                let mut id = None;
                let mut target = None;
                for attr in e.attributes().flatten() {
                    match attr.key.as_ref() {
                        b"Id" => id = Some(attr_value(&attr, reader.decoder())),
                        b"Target" => target = Some(attr_value(&attr, reader.decoder())),
                        _ => {}
                    }
                }
                if let (Some(id), Some(target)) = (id, target) {
                    out.insert(id, target);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

/// The default Office theme palette, in `theme=` attribute order (see
/// `parse_theme_palette`); used when theme1.xml cannot be read.
fn default_palette() -> Vec<(u8, u8, u8)> {
    vec![
        (0xFF, 0xFF, 0xFF), // lt1
        (0x00, 0x00, 0x00), // dk1
        (0xE7, 0xE6, 0xE6), // lt2
        (0x44, 0x54, 0x6A), // dk2
        (0x44, 0x72, 0xC4), // accent1
        (0xED, 0x7D, 0x31), // accent2
        (0xA5, 0xA5, 0xA5), // accent3
        (0xFF, 0xC0, 0x00), // accent4
        (0x5B, 0x9B, 0xD5), // accent5
        (0x70, 0xAD, 0x47), // accent6
        (0x05, 0x63, 0xC1), // hlink
        (0x95, 0x4F, 0x72), // folHlink
    ]
}

/// The `theme=` attribute indexes the clrScheme colors in display order,
/// which swaps each background/text pair relative to the file order:
/// 0=lt1, 1=dk1, 2=lt2, 3=dk2, then accent1-6, hlink, folHlink.
const THEME_ORDER: [&[u8]; 12] = [
    b"lt1",
    b"dk1",
    b"lt2",
    b"dk2",
    b"accent1",
    b"accent2",
    b"accent3",
    b"accent4",
    b"accent5",
    b"accent6",
    b"hlink",
    b"folHlink",
];

/// Reads `<a:clrScheme>` from theme1.xml: each named child holds either
/// `<a:srgbClr val="RRGGBB"/>` or `<a:sysClr … lastClr="RRGGBB"/>`.
fn parse_theme_palette(xml: &str) -> Result<Vec<(u8, u8, u8)>, MetaError> {
    let mut reader = Reader::from_str(xml);
    let mut in_scheme = false;
    let mut current: Option<Vec<u8>> = None;
    let mut named: HashMap<Vec<u8>, (u8, u8, u8)> = HashMap::new();
    loop {
        match reader.read_event().map_err(|e| MetaError(e.to_string()))? {
            Event::Start(e) if e.local_name().as_ref() == b"clrScheme" => in_scheme = true,
            Event::End(e) if e.local_name().as_ref() == b"clrScheme" => break,
            Event::Start(e) if in_scheme && THEME_ORDER.contains(&e.local_name().as_ref()) => {
                current = Some(e.local_name().as_ref().to_vec());
            }
            Event::Start(e) | Event::Empty(e) if in_scheme => {
                let attr_name = match e.local_name().as_ref() {
                    b"srgbClr" => b"val".as_ref(),
                    b"sysClr" => b"lastClr".as_ref(),
                    _ => continue,
                };
                let Some(name) = current.clone() else {
                    continue;
                };
                let rgb = e
                    .attributes()
                    .flatten()
                    .find(|a| a.key.as_ref() == attr_name)
                    .and_then(|a| parse_hex_rgb(&attr_value(&a, reader.decoder())));
                if let Some(rgb) = rgb {
                    named.entry(name).or_insert(rgb);
                }
            }
            Event::End(e) if THEME_ORDER.contains(&e.local_name().as_ref()) => current = None,
            Event::Eof => break,
            _ => {}
        }
    }
    if named.len() < THEME_ORDER.len() {
        return Err(MetaError("incomplete clrScheme".to_string()));
    }
    Ok(THEME_ORDER.iter().map(|name| named[*name]).collect())
}

/// `"RRGGBB"` or `"AARRGGBB"` (alpha ignored).
fn parse_hex_rgb(hex: &str) -> Option<(u8, u8, u8)> {
    // the byte-position slicing below panics mid-char on multi-byte input,
    // and these strings come straight from untrusted XML
    if !hex.is_ascii() {
        return None;
    }
    let rgb = match hex.len() {
        6 => hex,
        8 => &hex[2..],
        _ => return None,
    };
    let r = u8::from_str_radix(&rgb[0..2], 16).ok()?;
    let g = u8::from_str_radix(&rgb[2..4], 16).ok()?;
    let b = u8::from_str_radix(&rgb[4..6], 16).ok()?;
    Some((r, g, b))
}

/// Excel's tint: positive lightens toward white, negative darkens toward
/// black. (Officially defined on HSL luminance; this per-channel linear
/// approximation stays within a few units.)
fn apply_tint(rgb: (u8, u8, u8), tint: f64) -> (u8, u8, u8) {
    let tint = tint.clamp(-1.0, 1.0);
    let channel = |c: u8| -> u8 {
        let c = f64::from(c);
        let out = if tint >= 0.0 {
            c + (255.0 - c) * tint
        } else {
            c * (1.0 + tint)
        };
        out.round().clamp(0.0, 255.0) as u8
    };
    (channel(rgb.0), channel(rgb.1), channel(rgb.2))
}

/// One `CellStyle` per `cellXfs` entry (the style a cell's `s=` points at).
fn parse_styles(xml: &str, palette: &[(u8, u8, u8)]) -> Result<Vec<CellStyle>, MetaError> {
    let mut reader = Reader::from_str(xml);
    let mut custom: HashMap<u32, String> = HashMap::new();
    let mut fills: Vec<Option<(u8, u8, u8)>> = Vec::new();
    let mut fonts: Vec<Option<(u8, u8, u8)>> = Vec::new();
    let mut xfs: Vec<(u32, usize, usize)> = Vec::new();
    // `<numFmt>` also appears under `<dxfs>`, `<xf>` under `<cellStyleXfs>`,
    // `<fill>` and `<font>` under `<dxfs>`; only the `<numFmts>`, `<fills>`,
    // `<fonts>` and `<cellXfs>` sections define what cells reference.
    let mut in_num_fmts = false;
    let mut in_fills = false;
    let mut in_fonts = false;
    let mut in_cell_xfs = false;
    // fill state: set inside `<fill>` once a solid `<patternFill>` is seen
    let mut fill_depth = 0u32;
    let mut solid = false;
    let mut fill_color: Option<(u8, u8, u8)> = None;
    // font state: set inside `<font>` when its `<color>` child appears
    let mut font_depth = 0u32;
    let mut font_color: Option<(u8, u8, u8)> = None;
    loop {
        let event = reader.read_event().map_err(|e| MetaError(e.to_string()))?;
        match &event {
            Event::Start(e) | Event::Empty(e) => {
                let empty = matches!(event, Event::Empty(_));
                match e.local_name().as_ref() {
                    b"numFmts" if !empty => in_num_fmts = true,
                    b"fills" if !empty => in_fills = true,
                    b"fonts" if !empty => in_fonts = true,
                    b"cellXfs" if !empty => in_cell_xfs = true,
                    b"numFmt" if in_num_fmts => {
                        let mut id = None;
                        let mut code = None;
                        for attr in e.attributes().flatten() {
                            match attr.key.as_ref() {
                                b"numFmtId" => {
                                    id = attr_value(&attr, reader.decoder()).parse().ok()
                                }
                                b"formatCode" => code = Some(attr_value(&attr, reader.decoder())),
                                _ => {}
                            }
                        }
                        if let (Some(id), Some(code)) = (id, code) {
                            custom.insert(id, code);
                        }
                    }
                    b"fill" if in_fills => {
                        solid = false;
                        fill_color = None;
                        if empty {
                            fills.push(None);
                        } else {
                            fill_depth += 1;
                        }
                    }
                    b"patternFill" if in_fills && fill_depth > 0 => {
                        solid = e.attributes().flatten().any(|a| {
                            a.key.as_ref() == b"patternType"
                                && attr_value(&a, reader.decoder()) == "solid"
                        });
                    }
                    b"fgColor" if in_fills && fill_depth > 0 && solid => {
                        fill_color = parse_color_attrs(e, &reader, palette);
                    }
                    b"font" if in_fonts => {
                        font_color = None;
                        if empty {
                            fonts.push(None);
                        } else {
                            font_depth += 1;
                        }
                    }
                    b"color" if in_fonts && font_depth > 0 => {
                        font_color = parse_color_attrs(e, &reader, palette);
                    }
                    b"xf" if in_cell_xfs => {
                        let mut num_fmt = 0u32;
                        let mut fill_id = 0usize;
                        let mut font_id = 0usize;
                        for attr in e.attributes().flatten() {
                            match attr.key.as_ref() {
                                b"numFmtId" => {
                                    num_fmt =
                                        attr_value(&attr, reader.decoder()).parse().unwrap_or(0)
                                }
                                b"fillId" => {
                                    fill_id =
                                        attr_value(&attr, reader.decoder()).parse().unwrap_or(0)
                                }
                                b"fontId" => {
                                    font_id =
                                        attr_value(&attr, reader.decoder()).parse().unwrap_or(0)
                                }
                                _ => {}
                            }
                        }
                        xfs.push((num_fmt, fill_id, font_id));
                    }
                    _ => {}
                }
            }
            Event::End(e) => match e.local_name().as_ref() {
                b"numFmts" => in_num_fmts = false,
                b"fills" => in_fills = false,
                b"fonts" => in_fonts = false,
                b"cellXfs" => in_cell_xfs = false,
                b"fill" if in_fills && fill_depth > 0 => {
                    fill_depth -= 1;
                    fills.push(if solid { fill_color } else { None });
                }
                b"font" if in_fonts && font_depth > 0 => {
                    font_depth -= 1;
                    fonts.push(font_color);
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(xfs
        .into_iter()
        .map(|(num_fmt, fill_id, font_id)| CellStyle {
            format: match custom.get(&num_fmt) {
                Some(code) => Some(code.clone()),
                None => builtin_format(num_fmt).map(str::to_string),
            },
            fill: fills.get(fill_id).copied().flatten(),
            // font 0 is the workbook default — its color is the stock text
            // color (whatever the theme paints it), not an author's choice,
            // so inheriting it would restyle every plain cell
            font: if font_id == 0 {
                None
            } else {
                fonts.get(font_id).copied().flatten()
            },
        })
        .collect())
}

/// A color element's attributes: `rgb="FFRRGGBB"` wins, else `theme="n"`
/// (optionally tinted) through the palette. Legacy `indexed=` is ignored.
fn parse_color_attrs(
    e: &quick_xml::events::BytesStart,
    reader: &Reader<&[u8]>,
    palette: &[(u8, u8, u8)],
) -> Option<(u8, u8, u8)> {
    let mut rgb = None;
    let mut theme = None;
    let mut tint = 0.0f64;
    for attr in e.attributes().flatten() {
        let value = attr_value(&attr, reader.decoder());
        match attr.key.as_ref() {
            b"rgb" => rgb = parse_hex_rgb(&value),
            b"theme" => theme = value.parse::<usize>().ok(),
            b"tint" => tint = value.parse().unwrap_or(0.0),
            _ => {}
        }
    }
    if let Some(rgb) = rgb {
        return Some(rgb);
    }
    let base = palette.get(theme?).copied()?;
    Some(apply_tint(base, tint))
}

/// The numeric subset of the reserved built-in formats (ECMA-376 §18.8.30);
/// these are referenced by id only and never written into styles.xml.
/// Date/time ids (14-22, 45-48) and text (49) are intentionally absent so
/// those cells keep their existing rendering.
fn builtin_format(id: u32) -> Option<&'static str> {
    Some(match id {
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        9 => "0%",
        10 => "0.00%",
        37 => "#,##0;(#,##0)",
        38 => "#,##0;[Red](#,##0)",
        39 => "#,##0.00;(#,##0.00)",
        40 => "#,##0.00;[Red](#,##0.00)",
        41 => r#"_-* #,##0_-;-* #,##0_-;_-* "-"_-;_-@_-"#,
        42 => r#"_-"$"* #,##0_-;-"$"* #,##0_-;_-"$"* "-"_-;_-@_-"#,
        43 => r#"_-* #,##0.00_-;-* #,##0.00_-;_-* "-"??_-;_-@_-"#,
        44 => r#"_-"$"* #,##0.00_-;-"$"* #,##0.00_-;_-"$"* "-"??_-;_-@_-"#,
        _ => return None,
    })
}

/// `<c r="B2" s="5">` entries from a sheet XML, keeping only cells whose
/// style resolves to something visible. ECMA-376 makes both `<row r=…>` and
/// `<c r=…>` optional — positions then continue from the previous element —
/// so row and column are tracked explicitly.
fn parse_cell_styles(
    xml: &str,
    styles: &[CellStyle],
) -> Result<HashMap<(u32, u32), usize>, MetaError> {
    let mut reader = Reader::from_str(xml);
    let mut cells = HashMap::new();
    let mut row: Option<u32> = None;
    let mut next_col: u32 = 0;
    loop {
        match reader.read_event().map_err(|e| MetaError(e.to_string()))? {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"row" => {
                let explicit = e
                    .attributes()
                    .flatten()
                    .find(|a| a.key.as_ref() == b"r")
                    .and_then(|a| attr_value(&a, reader.decoder()).parse::<u32>().ok())
                    .and_then(|r| r.checked_sub(1));
                row = Some(explicit.unwrap_or_else(|| row.map_or(0, |r| r + 1)));
                next_col = 0;
            }
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"c" => {
                let mut reference = None;
                let mut style = None;
                for attr in e.attributes().flatten() {
                    match attr.key.as_ref() {
                        b"r" => reference = Some(attr_value(&attr, reader.decoder())),
                        b"s" => style = attr_value(&attr, reader.decoder()).parse::<usize>().ok(),
                        _ => {}
                    }
                }
                let position = match reference.as_deref().map(Anchor::parse_cell_ref) {
                    // an explicit reference also re-anchors the trackers
                    Some(Some((r, c))) => {
                        row = Some(r);
                        next_col = c + 1;
                        Some((r, c))
                    }
                    // a present but unparseable reference names no cell
                    Some(None) => None,
                    None => row.map(|r| {
                        let c = next_col;
                        next_col += 1;
                        (r, c)
                    }),
                };
                let (Some(position), Some(style)) = (position, style) else {
                    continue;
                };
                if styles.get(style).is_none_or(|s| s.is_plain()) {
                    continue;
                }
                cells.insert(position, style);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(cells)
}

/// `<col min="1" max="1" width="20.5"/>` entries from a sheet XML.
fn parse_cols(xml: &str) -> Result<Vec<ColumnWidth>, MetaError> {
    let mut reader = Reader::from_str(xml);
    let mut out = Vec::new();
    loop {
        match reader.read_event().map_err(|e| MetaError(e.to_string()))? {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"col" => {
                let mut min = None;
                let mut max = None;
                let mut width = None;
                for attr in e.attributes().flatten() {
                    match attr.key.as_ref() {
                        b"min" => min = attr_value(&attr, reader.decoder()).parse().ok(),
                        b"max" => max = attr_value(&attr, reader.decoder()).parse().ok(),
                        b"width" => width = attr_value(&attr, reader.decoder()).parse().ok(),
                        _ => {}
                    }
                }
                if let (Some(min), Some(max), Some(width)) = (min, max, width) {
                    out.push(ColumnWidth { min, max, width });
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn formats_of(styles: &[CellStyle]) -> Vec<Option<&str>> {
        styles.iter().map(|s| s.format.as_deref()).collect()
    }

    #[test]
    fn a_frozen_pane_yields_row_and_col_counts() {
        let xml = r#"<worksheet><sheetViews>
            <sheetView workbookViewId="0">
                <pane xSplit="1" ySplit="2" topLeftCell="B3" state="frozen"/>
            </sheetView>
        </sheetViews><sheetData/></worksheet>"#;
        assert_eq!(parse_pane(xml).unwrap(), Some((2, 1)));
    }

    #[test]
    fn frozen_split_counts_too_and_axes_may_be_absent() {
        let rows_only = r#"<sheetView><pane ySplit="1" state="frozenSplit"/></sheetView>"#;
        assert_eq!(parse_pane(rows_only).unwrap(), Some((1, 0)));
        let cols_only = r#"<sheetView><pane xSplit="3" state="frozen"/></sheetView>"#;
        assert_eq!(parse_pane(cols_only).unwrap(), Some((0, 3)));
    }

    #[test]
    fn non_frozen_splits_and_garbage_are_ignored() {
        // a plain split measures in twips, not cells — not a freeze
        let split = r#"<sheetView><pane xSplit="2310" ySplit="1050"/></sheetView>"#;
        assert_eq!(parse_pane(split).unwrap(), None);
        let garbage = r#"<sheetView><pane xSplit="NaN" ySplit="-3" state="frozen"/></sheetView>"#;
        assert_eq!(parse_pane(garbage).unwrap(), None);
        let none = r"<worksheet><sheetData/></worksheet>";
        assert_eq!(parse_pane(none).unwrap(), None);
    }

    #[test]
    fn styles_resolve_custom_and_builtin_ids() {
        let styles = r##"<styleSheet>
            <numFmts count="1">
                <numFmt numFmtId="164" formatCode="#,##0&quot;千円&quot;"/>
            </numFmts>
            <cellXfs count="4">
                <xf numFmtId="0"/>
                <xf numFmtId="9"/>
                <xf numFmtId="164"/>
                <xf numFmtId="999"/>
            </cellXfs>
        </styleSheet>"##;
        let styles = parse_styles(styles, &default_palette()).unwrap();
        assert_eq!(
            formats_of(&styles),
            vec![None, Some("0%"), Some("#,##0\"千円\""), None]
        );
        assert!(styles.iter().all(|s| s.fill.is_none()));
    }

    #[test]
    fn styles_ignore_cell_style_xfs_and_dxfs() {
        let styles = r#"<styleSheet>
            <dxfs count="1">
                <numFmt numFmtId="164" formatCode="0.000"/>
                <fill><patternFill patternType="solid"><fgColor rgb="FF123456"/></patternFill></fill>
            </dxfs>
            <cellStyleXfs count="1"><xf numFmtId="9"/></cellStyleXfs>
            <cellXfs count="1"><xf numFmtId="3"/></cellXfs>
        </styleSheet>"#;
        let styles = parse_styles(styles, &default_palette()).unwrap();
        assert_eq!(formats_of(&styles), vec![Some("#,##0")]);
        assert_eq!(styles[0].fill, None);
    }

    #[test]
    fn solid_fills_resolve_rgb_theme_and_tint() {
        let styles = r#"<styleSheet>
            <fills count="5">
                <fill><patternFill/></fill>
                <fill><patternFill patternType="gray125"/></fill>
                <fill><patternFill patternType="solid"><fgColor rgb="FFFF0000"/><bgColor indexed="64"/></patternFill></fill>
                <fill><patternFill patternType="solid"><fgColor theme="4"/></patternFill></fill>
                <fill><patternFill patternType="solid"><fgColor theme="4" tint="0.5"/></patternFill></fill>
            </fills>
            <cellXfs count="5">
                <xf fillId="0"/>
                <xf fillId="1"/>
                <xf fillId="2"/>
                <xf fillId="3"/>
                <xf fillId="4"/>
            </cellXfs>
        </styleSheet>"#;
        let styles = parse_styles(styles, &default_palette()).unwrap();
        let fills: Vec<_> = styles.iter().map(|s| s.fill).collect();
        // accent1 #4472C4; tint 0.5 lightens each channel halfway to white
        assert_eq!(
            fills,
            vec![
                None,
                None,
                Some((0xFF, 0x00, 0x00)),
                Some((0x44, 0x72, 0xC4)),
                Some((162, 185, 226)),
            ]
        );
    }

    #[test]
    fn theme_palette_reads_clr_scheme_in_display_order() {
        let theme = r#"<a:theme xmlns:a="x"><a:themeElements><a:clrScheme name="Office">
            <a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1>
            <a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>
            <a:dk2><a:srgbClr val="44546A"/></a:dk2>
            <a:lt2><a:srgbClr val="E7E6E6"/></a:lt2>
            <a:accent1><a:srgbClr val="112233"/></a:accent1>
            <a:accent2><a:srgbClr val="ED7D31"/></a:accent2>
            <a:accent3><a:srgbClr val="A5A5A5"/></a:accent3>
            <a:accent4><a:srgbClr val="FFC000"/></a:accent4>
            <a:accent5><a:srgbClr val="5B9BD5"/></a:accent5>
            <a:accent6><a:srgbClr val="70AD47"/></a:accent6>
            <a:hlink><a:srgbClr val="0563C1"/></a:hlink>
            <a:folHlink><a:srgbClr val="954F72"/></a:folHlink>
        </a:clrScheme></a:themeElements></a:theme>"#;
        let palette = parse_theme_palette(theme).unwrap();
        // display order swaps the pairs: 0=lt1 (white), 1=dk1 (black)
        assert_eq!(palette[0], (0xFF, 0xFF, 0xFF));
        assert_eq!(palette[1], (0x00, 0x00, 0x00));
        assert_eq!(palette[4], (0x11, 0x22, 0x33), "accent1 from the file");
    }

    #[test]
    fn truncated_theme_is_an_error() {
        let theme = r#"<a:theme xmlns:a="x"><a:clrScheme>
            <a:dk1><a:srgbClr val="000000"/></a:dk1>
        </a:clrScheme></a:theme>"#;
        assert!(parse_theme_palette(theme).is_err());
    }

    #[test]
    fn multibyte_hex_strings_are_rejected_not_a_panic() {
        // "Aあ12" is 6 bytes but slicing it at byte 2 would split あ
        assert_eq!(parse_hex_rgb("Aあ12"), None);
        assert_eq!(parse_hex_rgb("あdd12"), None, "8-byte variant");
        assert_eq!(parse_hex_rgb("FF0000"), Some((0xFF, 0, 0)));
        assert_eq!(parse_hex_rgb("00FF0000"), Some((0xFF, 0, 0)));
        assert_eq!(parse_hex_rgb("ZZ0000"), None, "non-hex ASCII");
    }

    #[test]
    fn an_empty_palette_drops_theme_fills_but_keeps_rgb() {
        let styles = r#"<styleSheet>
            <fills count="3">
                <fill><patternFill/></fill>
                <fill><patternFill patternType="solid"><fgColor rgb="FFFF0000"/></patternFill></fill>
                <fill><patternFill patternType="solid"><fgColor theme="4"/></patternFill></fill>
            </fills>
            <cellXfs count="2">
                <xf fillId="1"/>
                <xf fillId="2"/>
            </cellXfs>
        </styleSheet>"#;
        let styles = parse_styles(styles, &[]).unwrap();
        assert_eq!(styles[0].fill, Some((0xFF, 0x00, 0x00)));
        assert_eq!(styles[1].fill, None, "unresolvable theme paints nothing");
    }

    #[test]
    fn the_default_font_is_skipped_by_id_not_by_color() {
        // a theme may paint the default font a near-black like 0D0D0D:
        // matching on the color value would let every stock cell through
        let styles = r#"<styleSheet>
            <fonts count="2">
                <font><color rgb="FF0D0D0D"/></font>
                <font><color rgb="FFFF0000"/></font>
            </fonts>
            <cellXfs count="2">
                <xf fontId="0"/>
                <xf fontId="1"/>
            </cellXfs>
        </styleSheet>"#;
        let styles = parse_styles(styles, &default_palette()).unwrap();
        assert_eq!(styles[0].font, None, "font 0 is never an author's choice");
        assert_eq!(styles[1].font, Some((0xFF, 0x00, 0x00)));
    }

    #[test]
    fn multibyte_font_colors_are_rejected_not_a_panic() {
        // the same crafted-hex attack as fills, through the fonts entrance
        let styles = r#"<styleSheet>
            <fonts count="2">
                <font/>
                <font><color rgb="Aあ12"/></font>
            </fonts>
            <cellXfs count="1">
                <xf fontId="1"/>
            </cellXfs>
        </styleSheet>"#;
        let styles = parse_styles(styles, &default_palette()).unwrap();
        assert_eq!(styles[0].font, None);
    }

    #[test]
    fn tint_lightens_and_darkens() {
        assert_eq!(apply_tint((100, 100, 100), 0.0), (100, 100, 100));
        assert_eq!(apply_tint((100, 200, 0), 0.5), (178, 228, 128));
        assert_eq!(apply_tint((100, 200, 0), -0.5), (50, 100, 0));
        assert_eq!(apply_tint((10, 10, 10), 5.0), (255, 255, 255), "clamped");
    }

    #[test]
    fn date_builtins_are_not_resolved() {
        for id in 14..=22 {
            assert_eq!(builtin_format(id), None, "id {id} must stay unresolved");
        }
        assert_eq!(builtin_format(49), None, "text format must stay unresolved");
    }

    fn percent_style() -> Vec<CellStyle> {
        vec![
            CellStyle::default(),
            CellStyle {
                format: Some("0%".to_string()),
                fill: None,
                font: None,
            },
        ]
    }

    #[test]
    fn cell_styles_keep_only_cells_with_a_visible_style() {
        let sheet = r#"<worksheet><sheetData>
            <row r="1">
                <c r="A1" s="1"><v>0.15</v></c>
                <c r="B1" s="0"><v>1</v></c>
                <c r="C1"><v>2</v></c>
                <c r="D1" s="1"><v>0.25</v></c>
            </row>
        </sheetData></worksheet>"#;
        let cells = parse_cell_styles(sheet, &percent_style()).unwrap();
        assert_eq!(cells.get(&(0, 0)), Some(&1));
        assert_eq!(cells.get(&(0, 3)), Some(&1));
        assert!(!cells.contains_key(&(0, 1)), "plain style is dropped");
        assert!(!cells.contains_key(&(0, 2)), "no style at all");
    }

    #[test]
    fn cells_without_references_take_sequential_positions() {
        // ECMA-376 allows omitting r on both <row> and <c>; streaming
        // writers do exactly that
        let sheet = r#"<worksheet><sheetData>
            <row><c s="1"><v>1</v></c><c><v>2</v></c><c s="1"><v>3</v></c></row>
            <row r="5"><c s="1"><v>4</v></c></row>
            <row><c r="B6" s="1"/><c s="1"/></row>
        </sheetData></worksheet>"#;
        let cells = parse_cell_styles(sheet, &percent_style()).unwrap();
        let positions: Vec<(u32, u32)> = {
            let mut p: Vec<_> = cells.keys().copied().collect();
            p.sort_unstable();
            p
        };
        // row 0: A and C (B has no style); r="5" is row index 4;
        // then B6 re-anchors to (5, 1) and the next cell continues at (5, 2)
        assert_eq!(positions, vec![(0, 0), (0, 2), (4, 0), (5, 1), (5, 2)]);
    }

    #[test]
    fn out_of_range_style_indices_are_ignored() {
        let sheet = r#"<worksheet><sheetData>
            <row r="1"><c r="A1" s="7"><v>1</v></c></row>
        </sheetData></worksheet>"#;
        let cells = parse_cell_styles(sheet, &percent_style()).unwrap();
        assert!(cells.is_empty());
    }
}
