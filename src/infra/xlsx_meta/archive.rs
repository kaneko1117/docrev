use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

use quick_xml::Reader;
use quick_xml::events::Event;
use quick_xml::events::{BytesRef, BytesText};

use super::MetaError;
use crate::infra::ooxml;

pub(super) use crate::infra::ooxml::attr_value;

pub(super) fn open_archive(document: &Path) -> Result<zip::ZipArchive<File>, MetaError> {
    ooxml::open_archive(document).map_err(MetaError)
}

/// Targets are relative to `xl/`, or absolute with a leading `/`.
pub(super) fn entry_path(target: &str) -> String {
    match target.strip_prefix('/') {
        Some(absolute) => absolute.to_string(),
        None => format!("xl/{target}"),
    }
}

pub(super) fn read_entry(
    archive: &mut zip::ZipArchive<File>,
    name: &str,
) -> Result<String, MetaError> {
    ooxml::read_entry(archive, name).map_err(MetaError)
}

pub(super) fn text_piece(text: &BytesText) -> Result<String, MetaError> {
    ooxml::text_piece(text).map_err(MetaError)
}

pub(super) fn reference_piece(reference: &BytesRef) -> Result<String, MetaError> {
    ooxml::reference_piece(reference).map_err(MetaError)
}

/// `<sheet name="売上" r:id="rId1"/>` → (name, rId).
pub(super) fn parse_sheet_ids(xml: &str) -> Result<Vec<(String, String)>, MetaError> {
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

/// `<Relationship Id="rId1" Target="worksheets/sheet1.xml"/>` → (Id, Target).
pub(super) fn parse_rel_targets(xml: &str) -> Result<HashMap<String, String>, MetaError> {
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
