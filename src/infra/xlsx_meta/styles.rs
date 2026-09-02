//! styles.xml: what a cell's `s=` index resolves to, and which cells of a
//! sheet carry a style worth resolving.

use std::collections::HashMap;

use quick_xml::Reader;
use quick_xml::events::Event;

use crate::domain::anchor::Anchor;

use super::MetaError;
use super::archive::attr_value;
use super::theme::{apply_tint, parse_hex_rgb};

/// What a cell's `s=` style index resolves to: a number-format code, a
/// solid fill color and/or a font color (sRGB).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct CellStyle {
    pub format: Option<String>,
    pub fill: Option<(u8, u8, u8)>,
    pub font: Option<(u8, u8, u8)>,
}

impl CellStyle {
    pub(super) fn is_plain(&self) -> bool {
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

/// One `CellStyle` per `cellXfs` entry (the style a cell's `s=` points at).
pub(super) fn parse_styles(
    xml: &str,
    palette: &[(u8, u8, u8)],
) -> Result<Vec<CellStyle>, MetaError> {
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

/// The reserved built-in formats (ECMA-376 §18.8.30); these are referenced
/// by id only and never written into styles.xml. Ids 14-22 and the locale
/// block 27-36 are locale-dependent — docrev targets Japanese business
/// sheets, so they carry the ja-JP renderings. Scientific (11, 48) and
/// text (49) stay absent so those cells keep their existing rendering.
pub(super) fn builtin_format(id: u32) -> Option<&'static str> {
    Some(match id {
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        9 => "0%",
        10 => "0.00%",
        14 => "yyyy/m/d",
        15 => "d-mmm-yy",
        16 => "d-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "h:mm",
        21 => "h:mm:ss",
        22 => "yyyy/m/d h:mm",
        27 => "[$-411]ge.m.d",
        28 => "[$-411]ggge\"年\"m\"月\"d\"日\"",
        29 => "[$-411]ggge\"年\"m\"月\"d\"日\"",
        30 => "m/d/yy",
        31 => "yyyy\"年\"m\"月\"d\"日\"",
        32 => "h\"時\"mm\"分\"",
        33 => "h\"時\"mm\"分\"ss\"秒\"",
        34 => "yyyy\"年\"m\"月\"",
        35 => "m\"月\"d\"日\"",
        36 => "[$-411]ge.m.d",
        37 => "#,##0;(#,##0)",
        38 => "#,##0;[Red](#,##0)",
        39 => "#,##0.00;(#,##0.00)",
        40 => "#,##0.00;[Red](#,##0.00)",
        41 => r#"_-* #,##0_-;-* #,##0_-;_-* "-"_-;_-@_-"#,
        42 => r#"_-"$"* #,##0_-;-"$"* #,##0_-;_-"$"* "-"_-;_-@_-"#,
        43 => r#"_-* #,##0.00_-;-* #,##0.00_-;_-* "-"??_-;_-@_-"#,
        44 => r#"_-"$"* #,##0.00_-;-"$"* #,##0.00_-;_-"$"* "-"??_-;_-@_-"#,
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        // 47 (`mm:ss.0`) uses fractional seconds, which the engine degrades
        // to the fallback anyway — resolving it would change nothing
        _ => return None,
    })
}

/// `<c r="B2" s="5">` entries from a sheet XML, keeping only cells whose
/// style resolves to something visible. ECMA-376 makes both `<row r=…>` and
/// `<c r=…>` optional — positions then continue from the previous element —
/// so row and column are tracked explicitly.
pub(super) fn parse_cell_styles(
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
