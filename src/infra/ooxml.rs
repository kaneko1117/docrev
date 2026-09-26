//! Package plumbing shared by the xlsx and docx readers: the archive and XML text pieces.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use quick_xml::events::attributes::Attribute;
use quick_xml::events::{BytesCData, BytesRef, BytesText};

/// The message does not name the file; the caller adds the path once.
pub(crate) fn open_archive(document: &Path) -> Result<zip::ZipArchive<File>, String> {
    let file = File::open(document).map_err(|e| e.to_string())?;
    zip::ZipArchive::new(file).map_err(|e| e.to_string())
}

pub(crate) fn read_entry(
    archive: &mut zip::ZipArchive<File>,
    name: &str,
) -> Result<String, String> {
    let mut entry = archive.by_name(name).map_err(|e| format!("{name}: {e}"))?;
    let mut text = String::new();
    entry
        .read_to_string(&mut text)
        .map_err(|e| format!("{name}: {e}"))?;
    Ok(text)
}

pub(crate) fn attr_value(attr: &Attribute, decoder: quick_xml::Decoder) -> String {
    attr.decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, decoder)
        .map(|v| v.into_owned())
        .unwrap_or_default()
}

/// A text node arrives in pieces: plain text and each `&…;` reference as its own event.
pub(crate) fn text_piece(text: &BytesText) -> Result<String, String> {
    text.xml_content(quick_xml::XmlVersion::Implicit1_0)
        .map(|t| t.into_owned())
        .map_err(|e| e.to_string())
}

pub(crate) fn cdata_piece(cdata: &BytesCData) -> Result<String, String> {
    cdata
        .xml_content(quick_xml::XmlVersion::Implicit1_0)
        .map(|t| t.into_owned())
        .map_err(|e| e.to_string())
}

/// `&#x30;` and the predefined entities; an unknown entity is kept verbatim.
pub(crate) fn reference_piece(reference: &BytesRef) -> Result<String, String> {
    if let Some(ch) = reference.resolve_char_ref().map_err(|e| e.to_string())? {
        return Ok(ch.to_string());
    }
    let name = reference.decode().map_err(|e| e.to_string())?;
    let raw = format!("&{name};");
    Ok(quick_xml::escape::unescape(&raw)
        .map(|t| t.into_owned())
        .unwrap_or(raw))
}
