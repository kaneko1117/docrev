use std::path::Path;

use calamine::{Data, Range};

use crate::app::error::LoadError;
use crate::app::ports::DocumentSource;
use crate::domain::cell::CellValue;
use crate::domain::document::Document;
use crate::domain::sheet::Sheet;
use crate::infra::xlsx;

pub struct XlsxSource;

impl DocumentSource for XlsxSource {
    fn load(&self, path: &Path) -> Result<Document, LoadError> {
        let raw = xlsx::read_workbook(path).map_err(|e| LoadError(e.to_string()))?;
        let sheets = raw
            .into_iter()
            .map(|(name, range)| to_sheet(name, range))
            .collect();
        Ok(Document::new(sheets))
    }
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
