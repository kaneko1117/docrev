use std::path::Path;

use calamine::{Data, Range, Reader, Xlsx, open_workbook};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReadError {
    #[error("cannot open {path}: {source}")]
    Open {
        path: String,
        source: calamine::XlsxError,
    },
    #[error("cannot read sheet \"{name}\": {source}")]
    Sheet {
        name: String,
        source: calamine::XlsxError,
    },
}

/// One sheet as read from the file: values plus merged regions
/// (0-based inclusive (row, col) rectangles).
pub struct RawSheet {
    pub name: String,
    pub cells: Range<Data>,
    pub merges: Vec<((u32, u32), (u32, u32))>,
}

pub fn read_workbook(path: &Path) -> Result<Vec<RawSheet>, ReadError> {
    let mut workbook: Xlsx<_> = open_workbook(path).map_err(|source| ReadError::Open {
        path: path.display().to_string(),
        source,
    })?;
    let names = workbook.sheet_names().to_owned();
    let mut sheets = Vec::with_capacity(names.len());
    for name in names {
        let cells = workbook
            .worksheet_range(&name)
            .map_err(|source| ReadError::Sheet {
                name: name.clone(),
                source,
            })?;
        // merged regions are cosmetic — ignore load failures
        let merges = workbook
            .merge_cells_by_sheet_name(&name)
            .map(|dims| dims.into_iter().map(|d| (d.start, d.end)).collect())
            .unwrap_or_default();
        sheets.push(RawSheet {
            name,
            cells,
            merges,
        });
    }
    Ok(sheets)
}
