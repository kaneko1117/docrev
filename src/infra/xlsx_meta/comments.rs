use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::path::Path;

use quick_xml::Reader;
use quick_xml::events::Event;

use crate::domain::anchor::Anchor;

use super::MetaError;
use super::archive::{
    attr_value, entry_path, open_archive, parse_rel_targets, parse_sheet_ids, read_entry,
};

/// 0-based cell coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct RawWorkbookComment {
    pub row: u32,
    pub col: u32,
    pub author: String,
    pub body: String,
    pub resolved: bool,
    pub replies: Vec<(String, String)>,
}

/// Per sheet name. A cell with a threaded comment also carries a legacy fallback note; the note is dropped.
pub fn workbook_comments(
    document: &Path,
) -> Result<HashMap<String, Vec<RawWorkbookComment>>, MetaError> {
    let mut archive = open_archive(document)?;

    let workbook = read_entry(&mut archive, "xl/workbook.xml")?;
    let rels = read_entry(&mut archive, "xl/_rels/workbook.xml.rels")?;
    let sheets = parse_sheet_ids(&workbook)?;
    let targets = parse_rel_targets(&rels)?;
    // person GUID → display name
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
        let threaded_cells: HashSet<(u32, u32)> = comments.iter().map(|c| (c.row, c.col)).collect();
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

/// `worksheets/sheet1.xml` → `worksheets/_rels/sheet1.xml.rels`; `None` when absent.
fn read_sheet_rels(archive: &mut zip::ZipArchive<File>, sheet_path: &str) -> Option<String> {
    let (dir, file) = sheet_path.rsplit_once('/')?;
    read_entry(archive, &format!("{dir}/_rels/{file}.rels")).ok()
}

/// Targets whose relationship type ends with `/<kind>`, resolved against the worksheet's directory.
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

/// `<person displayName="田中" id="{GUID}"/>` → (id, displayName).
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

/// `<comment ref authorId>` with the body as its `<t>` runs; authors by index.
fn parse_legacy_comments(xml: &str) -> Result<Vec<RawWorkbookComment>, MetaError> {
    let mut reader = Reader::from_str(xml);
    let mut authors: Vec<String> = Vec::new();
    let mut out = Vec::new();
    let mut in_author = false;
    let mut current: Option<RawWorkbookComment> = None;
    let mut in_text = false;
    // <rPh> runs are ruby readings, not body text (山田 would read 山田ヤマダ)
    let mut in_phonetic = false;
    loop {
        match reader.read_event().map_err(|e| MetaError(e.to_string()))? {
            Event::Start(e) => match e.local_name().as_ref() {
                b"author" => in_author = true,
                b"rPh" => in_phonetic = true,
                b"comment" => {
                    let mut cell = None;
                    let mut author_id = None;
                    for attr in e.attributes().flatten() {
                        match attr.key.as_ref() {
                            b"ref" => {
                                cell = Anchor::parse_cell_ref(&attr_value(&attr, reader.decoder()))
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
                b"t" if current.is_some() && !in_phonetic => in_text = true,
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
                b"rPh" => in_phonetic = false,
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

/// Roots have no `parentId`; `done="1"` on the root maps to resolved.
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
    // (parent id, author, body)
    let mut replies: Vec<(String, String, String)> = Vec::new();
    let mut in_text = false;
    // Some(true) = a reply is open, Some(false) = a root is open
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
                        b"ref" => cell = Anchor::parse_cell_ref(&value),
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
