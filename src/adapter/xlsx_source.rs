use std::collections::HashMap;
use std::path::Path;

use calamine::{Data, Range};

use crate::app::error::LoadError;
use crate::app::ports::DocumentSource;
use crate::domain::cell::CellValue;
use crate::domain::document::Document;
use crate::domain::number_format::NumberFormat;
use crate::domain::sheet::{MergedRange, Rgb, Sheet};
use crate::infra::{xlsx, xlsx_meta};

pub struct XlsxSource;

impl DocumentSource for XlsxSource {
    fn load(&self, path: &Path) -> Result<Document, LoadError> {
        let raw = xlsx::read_workbook(path).map_err(|e| LoadError(e.to_string()))?;
        // widths, formats and fills are cosmetic — a parse failure must not
        // block opening; cells then simply show their raw, unpainted values
        let widths = xlsx_meta::column_widths(path).unwrap_or_default();
        let styles = xlsx_meta::cell_styles(path).unwrap_or_default();
        let frozen = xlsx_meta::frozen_panes(path).unwrap_or_default();
        // parse each distinct format once per workbook, not per cell
        let formats: Vec<Option<NumberFormat>> = styles
            .styles
            .iter()
            .map(|s| s.format.as_deref().map(NumberFormat::parse))
            .collect();
        let sheets = raw
            .into_iter()
            .map(|raw_sheet| {
                let cols = widths.get(&raw_sheet.name);
                let cells = styles.sheets.get(&raw_sheet.name);
                let merges: Vec<MergedRange> = raw_sheet
                    .merges
                    .iter()
                    .map(|(start, end)| MergedRange {
                        start_row: start.0 as usize,
                        start_col: start.1 as usize,
                        end_row: end.0 as usize,
                        end_col: end.1 as usize,
                    })
                    .collect();
                let (frozen_rows, frozen_cols) =
                    frozen.get(&raw_sheet.name).copied().unwrap_or((0, 0));
                let sheet = to_sheet(
                    raw_sheet.name,
                    raw_sheet.cells,
                    cells,
                    &styles.styles,
                    &formats,
                )
                .with_merges(merges)
                .with_frozen(frozen_rows, frozen_cols);
                match cols {
                    Some(cols) => {
                        let expanded = expand_widths(cols, sheet.col_count());
                        sheet.with_col_widths(expanded)
                    }
                    None => sheet,
                }
            })
            .collect();
        Ok(Document::new(sheets))
    }
}

/// Excel widths are float character counts on 1-based inclusive ranges;
/// terminal cells are integer columns, clamped to a sane range.
fn expand_widths(cols: &[xlsx_meta::ColumnWidth], col_count: usize) -> Vec<Option<u16>> {
    let mut widths = vec![None; col_count];
    for col in cols {
        // NaN slips through clamp (NaN.clamp() == NaN, as u16 == 0)
        if !col.width.is_finite() {
            continue;
        }
        let cells = (col.width.round().clamp(4.0, 60.0)) as u16;
        let from = col.min.saturating_sub(1) as usize;
        let to = (col.max as usize).min(col_count);
        for slot in widths.iter_mut().take(to).skip(from) {
            *slot = Some(cells);
        }
    }
    widths
}

/// Pads with `Empty` up to the used range's offset so row 0 / col 0 stay A1.
fn to_sheet(
    name: String,
    range: Range<Data>,
    cells: Option<&HashMap<(u32, u32), usize>>,
    styles: &[xlsx_meta::CellStyle],
    formats: &[Option<NumberFormat>],
) -> Sheet {
    let Some((start_row, start_col)) = range.start() else {
        return Sheet::new(name, Vec::new());
    };
    let mut rows: Vec<Vec<CellValue>> = Vec::with_capacity(start_row as usize + range.height());
    rows.resize_with(start_row as usize, Vec::new);
    for raw in range.rows() {
        let mut row = vec![CellValue::Empty; start_col as usize];
        row.extend(raw.iter().map(to_cell));
        rows.push(row);
    }
    let (fills, font_colors) = match cells {
        Some(cells) => apply_styles(&mut rows, cells, styles, formats),
        None => (HashMap::new(), HashMap::new()),
    };
    Sheet::new(name, rows)
        .with_fills(fills)
        .with_font_colors(font_colors)
}

type ColorMap = HashMap<(usize, usize), Rgb>;

/// Applies workbook styles to the freshly built grid. Numbers with a format
/// gain their display text (date cells were already converted by calamine
/// and text/bool cells have no numeric value, so only `Number` is touched);
/// fills and font colors are collected for the viewer, empty cells included.
fn apply_styles(
    rows: &mut [Vec<CellValue>],
    cells: &HashMap<(u32, u32), usize>,
    styles: &[xlsx_meta::CellStyle],
    formats: &[Option<NumberFormat>],
) -> (ColorMap, ColorMap) {
    let mut fills = HashMap::new();
    let mut font_colors = HashMap::new();
    for (&(row, col), &idx) in cells {
        let (row, col) = (row as usize, col as usize);
        let Some(style) = styles.get(idx) else {
            continue;
        };
        if let Some((r, g, b)) = style.fill {
            fills.insert((row, col), Rgb { r, g, b });
        }
        // the workbook-default font was already filtered out in infra
        // (by font id, not by color — a theme may paint the default any
        // near-black), so everything left is an author's choice
        if let Some((r, g, b)) = style.font {
            font_colors.insert((row, col), Rgb { r, g, b });
        }
        let Some(Some(format)) = formats.get(idx) else {
            continue;
        };
        if format.is_general() {
            continue;
        }
        let Some(cell) = rows.get_mut(row).and_then(|r| r.get_mut(col)) else {
            continue;
        };
        if let CellValue::Number(value) = *cell {
            let formatted = format.format(value);
            *cell = CellValue::FormattedNumber {
                value,
                text: formatted.text,
                color: formatted.color,
            };
        }
    }
    (fills, font_colors)
}

fn to_cell(data: &Data) -> CellValue {
    match data {
        Data::Empty => CellValue::Empty,
        Data::String(s) => CellValue::Text(s.clone()),
        Data::Float(f) => CellValue::Number(*f),
        Data::Int(i) => CellValue::Number(*i as f64),
        Data::Bool(b) => CellValue::Bool(*b),
        Data::DateTime(dt) => match dt.as_datetime() {
            Some(t) => CellValue::DateTime(t.to_string()),
            None => CellValue::Number(dt.as_f64()),
        },
        Data::DateTimeIso(s) => CellValue::DateTime(s.clone()),
        Data::DurationIso(s) => CellValue::Text(s.clone()),
        Data::Error(e) => CellValue::Error(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_finite_widths_are_ignored() {
        let cols = vec![
            xlsx_meta::ColumnWidth {
                min: 1,
                max: 1,
                width: f64::NAN,
            },
            xlsx_meta::ColumnWidth {
                min: 2,
                max: 2,
                width: f64::INFINITY,
            },
            xlsx_meta::ColumnWidth {
                min: 3,
                max: 3,
                width: 18.5,
            },
        ];
        assert_eq!(expand_widths(&cols, 3), vec![None, None, Some(19)]);
    }

    #[test]
    fn styles_format_numbers_collect_fills_and_keep_raw_values() {
        use crate::domain::number_format::FormatColor;

        let mut rows = vec![vec![
            CellValue::Number(0.15),
            CellValue::Number(-1234.0),
            CellValue::Number(3.0),
            CellValue::Text("x".into()),
            CellValue::Empty,
        ]];
        let styles = vec![
            xlsx_meta::CellStyle {
                format: Some("0%".into()),
                fill: Some((255, 255, 0)),
                font: None,
            },
            xlsx_meta::CellStyle {
                format: Some("#,##0;[Red]▲#,##0".into()),
                fill: None,
                font: None,
            },
            xlsx_meta::CellStyle {
                format: Some("General".into()),
                fill: None,
                font: Some((255, 255, 255)),
            },
            xlsx_meta::CellStyle {
                format: None,
                fill: Some((0, 128, 0)),
                font: Some((0, 0, 0)),
            },
        ];
        let formats: Vec<Option<NumberFormat>> = styles
            .iter()
            .map(|s| s.format.as_deref().map(NumberFormat::parse))
            .collect();
        let cells = [
            ((0u32, 0u32), 0usize),
            ((0, 1), 1),
            ((0, 2), 2),
            ((0, 3), 0),
            ((0, 4), 3),  // fill on an empty cell
            ((8, 25), 0), // outside the grid: must not panic
        ]
        .into();
        let (fills, font_colors) = apply_styles(&mut rows, &cells, &styles, &formats);
        assert_eq!(
            rows[0][0],
            CellValue::FormattedNumber {
                value: 0.15,
                text: "15%".into(),
                color: None,
            }
        );
        assert_eq!(
            rows[0][1],
            CellValue::FormattedNumber {
                value: -1234.0,
                text: "▲1,234".into(),
                color: Some(FormatColor::Red),
            }
        );
        assert_eq!(rows[0][2], CellValue::Number(3.0), "General stays raw");
        assert_eq!(rows[0][3], CellValue::Text("x".into()), "text is untouched");
        assert_eq!(
            fills.get(&(0, 0)),
            Some(&Rgb {
                r: 255,
                g: 255,
                b: 0
            })
        );
        assert_eq!(
            fills.get(&(0, 4)),
            Some(&Rgb { r: 0, g: 128, b: 0 }),
            "fills apply to empty cells too"
        );
        assert_eq!(fills.get(&(0, 1)), None);
        assert_eq!(
            font_colors.get(&(0, 2)),
            Some(&Rgb {
                r: 255,
                g: 255,
                b: 255
            }),
            "a real font color is inherited"
        );
        assert_eq!(
            font_colors.get(&(0, 4)),
            Some(&Rgb { r: 0, g: 0, b: 0 }),
            "an explicit black on a non-default font is an author's choice \
             (the workbook default is filtered by font id in infra)"
        );
    }
}
