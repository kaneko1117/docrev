use std::path::Path;

use calamine::{Data, Range};

use crate::app::error::LoadError;
use crate::app::ports::DocumentSource;
use crate::domain::cell::CellValue;
use crate::domain::document::Document;
use crate::domain::number_format::NumberFormat;
use crate::domain::sheet::{MergedRange, Sheet};
use crate::infra::{xlsx, xlsx_meta};

pub struct XlsxSource;

impl DocumentSource for XlsxSource {
    fn load(&self, path: &Path) -> Result<Document, LoadError> {
        let raw = xlsx::read_workbook(path).map_err(|e| LoadError(e.to_string()))?;
        // widths and formats are cosmetic — a parse failure must not block
        // opening; cells then simply show their raw values
        let widths = xlsx_meta::column_widths(path).unwrap_or_default();
        let formats = xlsx_meta::number_formats(path).unwrap_or_default();
        let sheets = raw
            .into_iter()
            .map(|raw_sheet| {
                let cols = widths.get(&raw_sheet.name);
                let fmts = formats.get(&raw_sheet.name);
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
                let sheet = to_sheet(raw_sheet.name, raw_sheet.cells, fmts).with_merges(merges);
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
fn to_sheet(name: String, range: Range<Data>, formats: Option<&xlsx_meta::SheetFormats>) -> Sheet {
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
    if let Some(formats) = formats {
        apply_formats(&mut rows, formats);
    }
    Sheet::new(name, rows)
}

/// Turns plain numbers into their formatted display where the workbook
/// assigns a format. Date cells were already converted by calamine and
/// text/bool cells have no numeric value, so only `Number` is touched.
fn apply_formats(rows: &mut [Vec<CellValue>], formats: &xlsx_meta::SheetFormats) {
    let parsed: Vec<NumberFormat> = formats
        .codes
        .iter()
        .map(|code| NumberFormat::parse(code))
        .collect();
    for (&(row, col), &idx) in &formats.cells {
        let Some(format) = parsed.get(idx) else {
            continue;
        };
        if format.is_general() {
            continue;
        }
        let Some(cell) = rows
            .get_mut(row as usize)
            .and_then(|r| r.get_mut(col as usize))
        else {
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
    fn formats_replace_plain_numbers_and_keep_raw_values() {
        use crate::domain::number_format::FormatColor;

        let mut rows = vec![vec![
            CellValue::Number(0.15),
            CellValue::Number(-1234.0),
            CellValue::Number(3.0),
            CellValue::Text("x".into()),
        ]];
        let formats = xlsx_meta::SheetFormats {
            codes: vec!["0%".into(), "#,##0;[Red]▲#,##0".into(), "General".into()],
            cells: [
                ((0, 0), 0),
                ((0, 1), 1),
                ((0, 2), 2),
                ((0, 3), 0),
                ((8, 25), 0), // outside the grid: must not panic
            ]
            .into(),
        };
        apply_formats(&mut rows, &formats);
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
    }
}
