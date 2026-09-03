use std::path::Path;

use calamine::{Data, Range, Reader, SheetVisible, Xlsx, open_workbook};
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

pub struct RawSheet {
    pub name: String,
    /// `hidden` or `veryHidden` in the workbook.
    pub hidden: bool,
    pub cells: Range<Data>,
    pub merges: Vec<((u32, u32), (u32, u32))>,
    /// Without the leading `=`; shared formulas already expanded per cell.
    pub formulas: Vec<((u32, u32), String)>,
}

pub struct RawWorkbook {
    pub sheets: Vec<RawSheet>,
    /// Serial dates count from 1904 instead of 1900.
    pub is_1904: bool,
}

pub fn read_workbook(path: &Path) -> Result<RawWorkbook, ReadError> {
    let mut workbook: Xlsx<_> = open_workbook(path).map_err(|source| ReadError::Open {
        path: path.display().to_string(),
        source,
    })?;
    let metadata: Vec<(String, bool)> = workbook
        .sheets_metadata()
        .iter()
        .map(|s| (s.name.clone(), s.visible != SheetVisible::Visible))
        .collect();
    let mut sheets = Vec::with_capacity(metadata.len());
    for (name, hidden) in metadata {
        let cells = workbook
            .worksheet_range(&name)
            .map_err(|source| ReadError::Sheet {
                name: name.clone(),
                source,
            })?;
        let merges = workbook
            .merge_cells_by_sheet_name(&name)
            .map(|dims| dims.into_iter().map(|d| (d.start, d.end)).collect())
            .unwrap_or_default();
        let formulas = workbook
            .worksheet_formula(&name)
            .map(|range| {
                // used_cells positions are relative to the range's corner
                let (start_row, start_col) = range.start().unwrap_or((0, 0));
                range
                    .used_cells()
                    .filter(|(_, _, f)| !f.is_empty())
                    .map(|(row, col, f)| {
                        ((start_row + row as u32, start_col + col as u32), f.clone())
                    })
                    .collect()
            })
            .unwrap_or_default();
        sheets.push(RawSheet {
            name,
            hidden,
            cells,
            merges,
            formulas,
        });
    }
    Ok(RawWorkbook {
        is_1904: workbook.has_1904_epoch(),
        sheets,
    })
}
