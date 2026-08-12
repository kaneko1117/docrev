use std::path::Path;

use calamine::{Data, Range};

use crate::app::error::LoadError;
use crate::app::ports::DocumentSource;
use crate::domain::cell::CellValue;
use crate::domain::document::Document;
use crate::domain::sheet::Sheet;
use crate::infra::{xlsx, xlsx_meta};

pub struct XlsxSource;

impl DocumentSource for XlsxSource {
    fn load(&self, path: &Path) -> Result<Document, LoadError> {
        let raw = xlsx::read_workbook(path).map_err(|e| LoadError(e.to_string()))?;
        // widths are cosmetic — a parse failure must not block opening
        let widths = xlsx_meta::column_widths(path).unwrap_or_default();
        let sheets = raw
            .into_iter()
            .map(|(name, range)| {
                let cols = widths.get(&name);
                let sheet = to_sheet(name, range);
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
fn to_sheet(name: String, range: Range<Data>) -> Sheet {
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
    Sheet::new(name, rows)
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
