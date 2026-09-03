use quick_xml::Reader;
use quick_xml::events::Event;

use super::MetaError;
use super::archive::attr_value;

/// 1-based inclusive column range; width in characters.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnRange {
    pub min: u32,
    pub max: u32,
    pub width: Option<f64>,
    pub hidden: bool,
}

/// (rows, cols) of the first frozen `<pane>`; non-frozen splits are in twips, not cells, and are ignored.
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
            // panes precede sheetData
            Event::Start(e) if e.local_name().as_ref() == b"sheetData" => return Ok(None),
            Event::Eof => return Ok(None),
            _ => {}
        }
    }
}

pub(super) fn parse_cols(xml: &str) -> Result<Vec<ColumnRange>, MetaError> {
    let mut reader = Reader::from_str(xml);
    let mut out = Vec::new();
    loop {
        match reader.read_event().map_err(|e| MetaError(e.to_string()))? {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"col" => {
                let mut min = None;
                let mut max = None;
                let mut width = None;
                let mut hidden = false;
                for attr in e.attributes().flatten() {
                    let value = attr_value(&attr, reader.decoder());
                    match attr.key.as_ref() {
                        b"min" => min = value.parse().ok(),
                        b"max" => max = value.parse().ok(),
                        b"width" => width = value.parse().ok(),
                        b"hidden" => hidden = is_true(&value),
                        _ => {}
                    }
                }
                if let (Some(min), Some(max)) = (min, max)
                    && (width.is_some() || hidden)
                {
                    out.push(ColumnRange {
                        min,
                        max,
                        width,
                        hidden,
                    });
                }
            }
            // cols precede sheetData
            Event::Start(e) if e.local_name().as_ref() == b"sheetData" => break,
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

/// 0-based indexes of `<row hidden="1">`; a `<row>` without `r` follows the previous one.
pub(super) fn parse_hidden_rows(xml: &str) -> Result<Vec<u32>, MetaError> {
    let mut reader = Reader::from_str(xml);
    let mut out = Vec::new();
    let mut row: Option<u32> = None;
    loop {
        match reader.read_event().map_err(|e| MetaError(e.to_string()))? {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"row" => {
                let mut explicit = None;
                let mut hidden = false;
                for attr in e.attributes().flatten() {
                    let value = attr_value(&attr, reader.decoder());
                    match attr.key.as_ref() {
                        b"r" => explicit = value.parse::<u32>().ok().and_then(|r| r.checked_sub(1)),
                        b"hidden" => hidden = is_true(&value),
                        _ => {}
                    }
                }
                // a row past u32::MAX cannot exist; stop counting rather than wrap to 0
                let Some(current) = explicit.or_else(|| row.map_or(Some(0), |r| r.checked_add(1)))
                else {
                    break;
                };
                row = Some(current);
                if hidden {
                    out.push(current);
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"sheetData" => break,
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

fn is_true(value: &str) -> bool {
    value == "1" || value == "true"
}
