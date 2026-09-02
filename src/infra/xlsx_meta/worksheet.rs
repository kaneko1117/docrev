//! Per-worksheet attributes: `<cols>` widths and the frozen `<pane>`.

use quick_xml::Reader;
use quick_xml::events::Event;

use super::MetaError;
use super::archive::attr_value;

/// 1-based column range with its Excel width (in characters).
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnWidth {
    pub min: u32,
    pub max: u32,
    pub width: f64,
}

/// The first frozen `<pane>`. Non-frozen splits measure their offsets in
/// twips, not cells, so anything without a frozen state is ignored; frozen
/// splits carry whole cell counts in `xSplit`/`ySplit`.
pub(super) fn parse_pane(xml: &str) -> Result<Option<(usize, usize)>, MetaError> {
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

/// `<col min="1" max="1" width="20.5"/>` entries from a sheet XML.
pub(super) fn parse_cols(xml: &str) -> Result<Vec<ColumnWidth>, MetaError> {
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
