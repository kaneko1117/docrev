use std::collections::{HashMap, HashSet};

use quick_xml::Reader;
use quick_xml::events::Event;

use crate::domain::anchor::Anchor;

use super::MetaError;
use super::archive::attr_value;
use super::theme::{apply_tint, parse_hex_rgb};

/// What a cell's `s=` index resolves to; colors are sRGB.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct CellStyle {
    pub format: Option<String>,
    pub fill: Option<(u8, u8, u8)>,
    pub font: Option<(u8, u8, u8)>,
    pub alignment: Option<CellAlignment>,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
}

/// `<alignment>` attributes as written; `horizontal` and `vertical` are the file's keywords.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct CellAlignment {
    pub horizontal: Option<String>,
    pub vertical: Option<String>,
    pub indent: u32,
}

impl CellStyle {
    pub(super) fn is_plain(&self) -> bool {
        self.format.is_none()
            && self.fill.is_none()
            && self.font.is_none()
            && self.alignment.is_none()
            && !(self.bold || self.italic || self.strike)
    }
}

#[derive(Debug, Default, Clone)]
pub struct WorkbookStyles {
    pub styles: Vec<CellStyle>,
    pub sheets: HashMap<String, SheetCells>,
}

/// 0-based (row, col): `styled` maps to indices in `styles`; `blank` holds the `<c/>` elements
/// written without content, which carry their own style instead of their row's or column's.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct SheetCells {
    pub styled: HashMap<(u32, u32), usize>,
    pub blank: HashSet<(u32, u32)>,
}

impl SheetCells {
    pub fn is_empty(&self) -> bool {
        self.styled.is_empty() && self.blank.is_empty()
    }
}

/// One entry per `cellXfs` `<xf>`.
pub(super) fn parse_styles(
    xml: &str,
    palette: &[(u8, u8, u8)],
) -> Result<Vec<CellStyle>, MetaError> {
    // <colors> comes after <fonts> and <fills> in the file, so the indexed palette is read first
    let indexed = parse_indexed_palette(xml)?;
    let palettes = Palettes {
        theme: palette,
        indexed: &indexed,
    };
    let palette = &palettes;
    let mut reader = Reader::from_str(xml);
    let mut custom: HashMap<u32, String> = HashMap::new();
    let mut fills: Vec<Option<(u8, u8, u8)>> = Vec::new();
    let mut fonts: Vec<FontFace> = Vec::new();
    let mut xfs: Vec<(u32, usize, usize, Option<CellAlignment>)> = Vec::new();
    // the same elements also appear under <dxfs> and <cellStyleXfs>, which cells never reference
    let mut in_num_fmts = false;
    let mut in_fills = false;
    let mut in_fonts = false;
    let mut in_cell_xfs = false;
    let mut in_xf = false;
    let mut fill_depth = 0u32;
    let mut solid = false;
    let mut fill_color: Option<(u8, u8, u8)> = None;
    let mut font_depth = 0u32;
    let mut face = FontFace::default();
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
                        face = FontFace::default();
                        if empty {
                            fonts.push(face);
                        } else {
                            font_depth += 1;
                        }
                    }
                    b"color" if in_fonts && font_depth > 0 => {
                        face.color = parse_color_attrs(e, &reader, palette);
                    }
                    b"b" if in_fonts && font_depth > 0 => face.bold = flag_on(e, &reader),
                    b"i" if in_fonts && font_depth > 0 => face.italic = flag_on(e, &reader),
                    b"strike" if in_fonts && font_depth > 0 => face.strike = flag_on(e, &reader),
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
                        xfs.push((num_fmt, fill_id, font_id, None));
                        in_xf = !empty;
                    }
                    b"alignment" if in_xf => {
                        let mut alignment: CellAlignment = CellAlignment::default();
                        for attr in e.attributes().flatten() {
                            let value = attr_value(&attr, reader.decoder());
                            match attr.key.as_ref() {
                                b"horizontal" => alignment.horizontal = Some(value),
                                b"vertical" => alignment.vertical = Some(value),
                                b"indent" => alignment.indent = value.parse().unwrap_or(0),
                                _ => {}
                            }
                        }
                        if alignment != CellAlignment::default()
                            && let Some(xf) = xfs.last_mut()
                        {
                            xf.3 = Some(alignment);
                        }
                    }
                    _ => {}
                }
            }
            Event::End(e) => match e.local_name().as_ref() {
                b"numFmts" => in_num_fmts = false,
                b"fills" => in_fills = false,
                b"fonts" => in_fonts = false,
                b"cellXfs" => in_cell_xfs = false,
                b"xf" => in_xf = false,
                b"fill" if in_fills && fill_depth > 0 => {
                    fill_depth -= 1;
                    fills.push(if solid { fill_color } else { None });
                }
                b"font" if in_fonts && font_depth > 0 => {
                    font_depth -= 1;
                    fonts.push(face);
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    let face_of = |font_id: usize| {
        if font_id == 0 {
            FontFace::default()
        } else {
            fonts.get(font_id).copied().unwrap_or_default()
        }
    };
    Ok(xfs
        .into_iter()
        .map(|(num_fmt, fill_id, font_id, alignment)| CellStyle {
            format: match custom.get(&num_fmt) {
                Some(code) => Some(code.clone()),
                None => builtin_format(num_fmt).map(str::to_string),
            },
            fill: fills.get(fill_id).copied().flatten(),
            // font 0 is the workbook default; inheriting it would restyle every plain cell
            font: face_of(font_id).color,
            alignment,
            bold: face_of(font_id).bold,
            italic: face_of(font_id).italic,
            strike: face_of(font_id).strike,
        })
        .collect())
}

/// One `<fonts>` entry; color is sRGB.
#[derive(Debug, Default, Clone, Copy)]
struct FontFace {
    color: Option<(u8, u8, u8)>,
    bold: bool,
    italic: bool,
    strike: bool,
}

/// A bare `<b/>` is on; `val="0"` or `val="false"` turns it off.
fn flag_on(e: &quick_xml::events::BytesStart, reader: &Reader<&[u8]>) -> bool {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == b"val")
        .is_none_or(|a| {
            let value = attr_value(&a, reader.decoder());
            value != "0" && value != "false"
        })
}

/// Theme colors in `theme=` order; legacy colors in `indexed=` order.
struct Palettes<'a> {
    theme: &'a [(u8, u8, u8)],
    indexed: &'a [(u8, u8, u8)],
}

/// `rgb=` wins, then `indexed=`, then `theme=` (+ `tint=`); `auto="1"` and indexes past the
/// palette (64 and 65 are the system colors) are no color.
fn parse_color_attrs(
    e: &quick_xml::events::BytesStart,
    reader: &Reader<&[u8]>,
    palettes: &Palettes,
) -> Option<(u8, u8, u8)> {
    let mut rgb = None;
    let mut indexed = None;
    let mut theme = None;
    let mut tint = 0.0f64;
    for attr in e.attributes().flatten() {
        let value = attr_value(&attr, reader.decoder());
        match attr.key.as_ref() {
            b"rgb" => rgb = parse_hex_rgb(&value),
            b"indexed" => indexed = value.parse::<usize>().ok(),
            b"theme" => theme = value.parse::<usize>().ok(),
            b"tint" => tint = value.parse().unwrap_or(0.0),
            b"auto" if value == "1" || value == "true" => return None,
            _ => {}
        }
    }
    if let Some(rgb) = rgb {
        return Some(rgb);
    }
    if let Some(index) = indexed {
        return palettes.indexed.get(index).copied();
    }
    let base = palettes.theme.get(theme?).copied()?;
    Some(apply_tint(base, tint))
}

/// ECMA-376 §18.8.27: the legacy palette in `indexed=` order.
const INDEXED_PALETTE: [u32; 64] = [
    0x000000, 0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, 0x000000,
    0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, 0x800000, 0x008000,
    0x000080, 0x808000, 0x800080, 0x008080, 0xC0C0C0, 0x808080, 0x9999FF, 0x993366, 0xFFFFCC,
    0xCCFFFF, 0x660066, 0xFF8080, 0x0066CC, 0xCCCCFF, 0x000080, 0xFF00FF, 0xFFFF00, 0x00FFFF,
    0x800080, 0x800000, 0x008080, 0x0000FF, 0x00CCFF, 0xCCFFFF, 0xCCFFCC, 0xFFFF99, 0x99CCFF,
    0xFF99CC, 0xCC99FF, 0xFFCC99, 0x3366FF, 0x33CCCC, 0x99CC00, 0xFFCC00, 0xFF9900, 0xFF6600,
    0x666699, 0x969696, 0x003366, 0x339966, 0x003300, 0x333300, 0x993300, 0x993366, 0x333399,
    0x333333,
];

fn default_indexed_palette() -> Vec<(u8, u8, u8)> {
    INDEXED_PALETTE
        .iter()
        .map(|&c| ((c >> 16) as u8, (c >> 8) as u8, c as u8))
        .collect()
}

/// The workbook's own `<indexedColors>` when it has one, else the default legacy palette.
fn parse_indexed_palette(xml: &str) -> Result<Vec<(u8, u8, u8)>, MetaError> {
    let mut reader = Reader::from_str(xml);
    let mut in_indexed = false;
    let mut custom = Vec::new();
    loop {
        match reader.read_event().map_err(|e| MetaError(e.to_string()))? {
            Event::Start(e) if e.local_name().as_ref() == b"indexedColors" => in_indexed = true,
            Event::End(e) if e.local_name().as_ref() == b"indexedColors" => break,
            Event::Start(e) | Event::Empty(e)
                if in_indexed && e.local_name().as_ref() == b"rgbColor" =>
            {
                // a missing or broken entry keeps its slot so later indexes stay aligned
                let rgb = e
                    .attributes()
                    .flatten()
                    .find(|a| a.key.as_ref() == b"rgb")
                    .and_then(|a| parse_hex_rgb(&attr_value(&a, reader.decoder())));
                custom.push(rgb.unwrap_or((0, 0, 0)));
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(if custom.is_empty() {
        default_indexed_palette()
    } else {
        custom
    })
}

/// Built-in ids (ECMA-376 §18.8.30) with ja-JP renderings for the locale-dependent ones; scientific (11, 48) and text (49) stay `None`.
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
        // 47 (`mm:ss.0`): fractional seconds degrade to the fallback anyway
        _ => return None,
    })
}

/// Cells whose style is not plain. `<row r=…>` and `<c r=…>` are optional; positions then continue from the previous element.
pub(super) fn parse_cell_styles(xml: &str, styles: &[CellStyle]) -> Result<SheetCells, MetaError> {
    let mut reader = Reader::from_str(xml);
    let mut cells = SheetCells::default();
    let mut row: Option<u32> = None;
    let mut next_col: u32 = 0;
    loop {
        let event = reader.read_event().map_err(|e| MetaError(e.to_string()))?;
        match &event {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"row" => {
                let explicit = e
                    .attributes()
                    .flatten()
                    .find(|a| a.key.as_ref() == b"r")
                    .and_then(|a| attr_value(&a, reader.decoder()).parse::<u32>().ok())
                    .and_then(|r| r.checked_sub(1));
                // a row past u32::MAX cannot exist; stop rather than wrap to 0
                let Some(current) = explicit.or_else(|| row.map_or(Some(0), |r| r.checked_add(1)))
                else {
                    break;
                };
                row = Some(current);
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
                    Some(Some((r, c))) => {
                        row = Some(r);
                        next_col = c + 1;
                        Some((r, c))
                    }
                    Some(None) => None,
                    None => row.map(|r| {
                        let c = next_col;
                        next_col += 1;
                        (r, c)
                    }),
                };
                let Some(position) = position else {
                    continue;
                };
                // a childless <c/> holds no value: present in the file, yet nothing to show
                if matches!(event, Event::Empty(_)) {
                    cells.blank.insert(position);
                }
                let Some(style) = style else {
                    continue;
                };
                if styles.get(style).is_none_or(|s| s.is_plain()) {
                    continue;
                }
                cells.styled.insert(position, style);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(cells)
}
