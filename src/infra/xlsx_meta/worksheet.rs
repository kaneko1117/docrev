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

/// 0-based; `height` is set only for `customHeight` rows, in points.
#[derive(Debug, Clone, PartialEq)]
pub struct RowAttrs {
    pub index: u32,
    pub hidden: bool,
    pub height: Option<f64>,
}

/// Character width and points, each only when the file states a positive finite value.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SheetFormat {
    pub default_col_width: Option<f64>,
    pub default_row_height: Option<f64>,
}

/// Excel derives `defaultColWidth` from `baseColWidth` by adding cell padding.
const BASE_COL_PADDING: f64 = 0.71;
/// The spec's default `baseColWidth`; writers restate it, so it carries no intent.
const SPEC_BASE_COL_WIDTH: f64 = 8.0;

pub(super) fn parse_sheet_format(xml: &str) -> Result<SheetFormat, MetaError> {
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event().map_err(|e| MetaError(e.to_string()))? {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"sheetFormatPr" => {
                let mut default_col_width = None;
                let mut base_col_width = None;
                let mut default_row_height = None;
                for attr in e.attributes().flatten() {
                    let value = attr_value(&attr, reader.decoder());
                    match attr.key.as_ref() {
                        b"defaultColWidth" => default_col_width = positive(&value),
                        b"baseColWidth" => base_col_width = positive(&value),
                        b"defaultRowHeight" => default_row_height = positive(&value),
                        _ => {}
                    }
                }
                return Ok(SheetFormat {
                    default_col_width: default_col_width.or(base_col_width
                        .filter(|w| *w != SPEC_BASE_COL_WIDTH)
                        .map(|w| w + BASE_COL_PADDING)),
                    default_row_height,
                });
            }
            // sheetFormatPr precedes sheetData
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"sheetData" => break,
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(SheetFormat::default())
}

/// Rows that are hidden or have a custom height; a `<row>` without `r` follows the previous one.
pub(super) fn parse_rows(xml: &str) -> Result<Vec<RowAttrs>, MetaError> {
    let mut reader = Reader::from_str(xml);
    let mut out = Vec::new();
    let mut row: Option<u32> = None;
    loop {
        match reader.read_event().map_err(|e| MetaError(e.to_string()))? {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"row" => {
                let mut explicit = None;
                let mut hidden = false;
                let mut height = None;
                let mut custom = false;
                for attr in e.attributes().flatten() {
                    let value = attr_value(&attr, reader.decoder());
                    match attr.key.as_ref() {
                        b"r" => explicit = value.parse::<u32>().ok().and_then(|r| r.checked_sub(1)),
                        b"hidden" => hidden = is_true(&value),
                        b"ht" => height = positive(&value),
                        b"customHeight" => custom = is_true(&value),
                        _ => {}
                    }
                }
                // a row past u32::MAX cannot exist; stop counting rather than wrap to 0
                let Some(current) = explicit.or_else(|| row.map_or(Some(0), |r| r.checked_add(1)))
                else {
                    break;
                };
                row = Some(current);
                let height = height.filter(|_| custom);
                if hidden || height.is_some() {
                    out.push(RowAttrs {
                        index: current,
                        hidden,
                        height,
                    });
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"sheetData" => break,
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

/// `None` unless the value parses to a positive finite number.
fn positive(value: &str) -> Option<f64> {
    value
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v > 0.0)
}

fn is_true(value: &str) -> bool {
    value == "1" || value == "true"
}
