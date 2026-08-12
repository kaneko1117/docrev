use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use quick_xml::Reader;
use quick_xml::events::Event;
use quick_xml::events::attributes::Attribute;
use thiserror::Error;

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
    let file =
        File::open(document).map_err(|e| MetaError(format!("{}: {e}", document.display())))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| MetaError(e.to_string()))?;

    let workbook = read_entry(&mut archive, "xl/workbook.xml")?;
    let rels = read_entry(&mut archive, "xl/_rels/workbook.xml.rels")?;
    let sheets = parse_sheet_ids(&workbook)?;
    let targets = parse_rel_targets(&rels)?;

    let mut result = HashMap::new();
    for (name, rid) in sheets {
        let Some(target) = targets.get(&rid) else {
            continue;
        };
        let path = match target.strip_prefix('/') {
            Some(absolute) => absolute.to_string(),
            None => format!("xl/{target}"),
        };
        let xml = read_entry(&mut archive, &path)?;
        let cols = parse_cols(&xml)?;
        if !cols.is_empty() {
            result.insert(name, cols);
        }
    }
    Ok(result)
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
