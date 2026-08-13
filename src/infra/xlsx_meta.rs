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

/// Number-format codes per sheet: `cells` maps A1 refs to indices in `codes`.
#[derive(Debug, Default, Clone)]
pub struct SheetFormats {
    pub codes: Vec<String>,
    pub cells: HashMap<String, usize>,
}

/// Per-cell number-format codes, resolved through styles.xml
/// (`numFmts` + `cellXfs`) and each sheet's `s=` style indices.
/// calamine does not expose formats either.
pub fn number_formats(document: &Path) -> Result<HashMap<String, SheetFormats>, MetaError> {
    let mut archive = open_archive(document)?;

    let styles = read_entry(&mut archive, "xl/styles.xml")?;
    let style_codes = parse_style_codes(&styles)?;
    if style_codes.iter().all(Option::is_none) {
        return Ok(HashMap::new());
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
        let xml = read_entry(&mut archive, &entry_path(target))?;
        let formats = parse_cell_formats(&xml, &style_codes)?;
        if !formats.cells.is_empty() {
            result.insert(name, formats);
        }
    }
    Ok(result)
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

/// One format code per `cellXfs` entry (the style a cell's `s=` points at),
/// `None` where the style has no format we can express as a code.
fn parse_style_codes(xml: &str) -> Result<Vec<Option<String>>, MetaError> {
    let mut reader = Reader::from_str(xml);
    let mut custom: HashMap<u32, String> = HashMap::new();
    let mut xf_ids: Vec<u32> = Vec::new();
    // `<numFmt>` also appears under `<dxfs>` and `<xf>` under `<cellStyleXfs>`;
    // only the `<numFmts>` and `<cellXfs>` sections define what cells reference.
    let mut in_num_fmts = false;
    let mut in_cell_xfs = false;
    loop {
        match reader.read_event().map_err(|e| MetaError(e.to_string()))? {
            Event::Start(e) => match e.local_name().as_ref() {
                b"numFmts" => in_num_fmts = true,
                b"cellXfs" => in_cell_xfs = true,
                b"numFmt" if in_num_fmts => collect_num_fmt(&e, &reader, &mut custom),
                b"xf" if in_cell_xfs => xf_ids.push(xf_num_fmt_id(&e, &reader)),
                _ => {}
            },
            Event::Empty(e) => match e.local_name().as_ref() {
                b"numFmt" if in_num_fmts => collect_num_fmt(&e, &reader, &mut custom),
                b"xf" if in_cell_xfs => xf_ids.push(xf_num_fmt_id(&e, &reader)),
                _ => {}
            },
            Event::End(e) => match e.local_name().as_ref() {
                b"numFmts" => in_num_fmts = false,
                b"cellXfs" => in_cell_xfs = false,
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(xf_ids
        .into_iter()
        .map(|id| match custom.get(&id) {
            Some(code) => Some(code.clone()),
            None => builtin_format(id).map(str::to_string),
        })
        .collect())
}

fn collect_num_fmt(
    e: &quick_xml::events::BytesStart,
    reader: &Reader<&[u8]>,
    custom: &mut HashMap<u32, String>,
) {
    let mut id = None;
    let mut code = None;
    for attr in e.attributes().flatten() {
        match attr.key.as_ref() {
            b"numFmtId" => id = attr_value(&attr, reader.decoder()).parse().ok(),
            b"formatCode" => code = Some(attr_value(&attr, reader.decoder())),
            _ => {}
        }
    }
    if let (Some(id), Some(code)) = (id, code) {
        custom.insert(id, code);
    }
}

fn xf_num_fmt_id(e: &quick_xml::events::BytesStart, reader: &Reader<&[u8]>) -> u32 {
    for attr in e.attributes().flatten() {
        if attr.key.as_ref() == b"numFmtId" {
            return attr_value(&attr, reader.decoder()).parse().unwrap_or(0);
        }
    }
    0
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
/// style resolves to a format code.
fn parse_cell_formats(
    xml: &str,
    style_codes: &[Option<String>],
) -> Result<SheetFormats, MetaError> {
    let mut reader = Reader::from_str(xml);
    let mut codes: Vec<String> = Vec::new();
    let mut code_index: HashMap<usize, usize> = HashMap::new();
    let mut cells = HashMap::new();
    loop {
        match reader.read_event().map_err(|e| MetaError(e.to_string()))? {
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
                let (Some(reference), Some(style)) = (reference, style) else {
                    continue;
                };
                let Some(Some(code)) = style_codes.get(style) else {
                    continue;
                };
                let idx = *code_index.entry(style).or_insert_with(|| {
                    codes.push(code.clone());
                    codes.len() - 1
                });
                cells.insert(reference, idx);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(SheetFormats { codes, cells })
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

    #[test]
    fn style_codes_resolve_custom_and_builtin_ids() {
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
        let codes = parse_style_codes(styles).unwrap();
        assert_eq!(
            codes,
            vec![
                None,
                Some("0%".to_string()),
                Some("#,##0\"千円\"".to_string()),
                None,
            ]
        );
    }

    #[test]
    fn style_codes_ignore_cell_style_xfs_and_dxfs() {
        let styles = r#"<styleSheet>
            <dxfs count="1"><numFmt numFmtId="164" formatCode="0.000"/></dxfs>
            <cellStyleXfs count="1"><xf numFmtId="9"/></cellStyleXfs>
            <cellXfs count="1"><xf numFmtId="3"/></cellXfs>
        </styleSheet>"#;
        let codes = parse_style_codes(styles).unwrap();
        assert_eq!(codes, vec![Some("#,##0".to_string())]);
    }

    #[test]
    fn date_builtins_are_not_resolved() {
        for id in 14..=22 {
            assert_eq!(builtin_format(id), None, "id {id} must stay unresolved");
        }
        assert_eq!(builtin_format(49), None, "text format must stay unresolved");
    }

    #[test]
    fn cell_formats_keep_only_cells_with_a_resolvable_style() {
        let sheet = r#"<worksheet><sheetData>
            <row r="1">
                <c r="A1" s="1"><v>0.15</v></c>
                <c r="B1" s="0"><v>1</v></c>
                <c r="C1"><v>2</v></c>
                <c r="D1" s="1"><v>0.25</v></c>
            </row>
        </sheetData></worksheet>"#;
        let codes = vec![None, Some("0%".to_string())];
        let formats = parse_cell_formats(sheet, &codes).unwrap();
        assert_eq!(formats.codes, vec!["0%".to_string()]);
        assert_eq!(formats.cells.get("A1"), Some(&0));
        assert_eq!(formats.cells.get("D1"), Some(&0));
        assert!(!formats.cells.contains_key("B1"));
        assert!(!formats.cells.contains_key("C1"));
    }

    #[test]
    fn out_of_range_style_indices_are_ignored() {
        let sheet = r#"<worksheet><sheetData>
            <row r="1"><c r="A1" s="7"><v>1</v></c></row>
        </sheetData></worksheet>"#;
        let formats = parse_cell_formats(sheet, &[Some("0%".to_string())]).unwrap();
        assert!(formats.cells.is_empty());
    }
}
