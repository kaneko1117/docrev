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

pub fn read_workbook(path: &Path) -> Result<Vec<(String, Range<Data>)>, ReadError> {
    let mut workbook: Xlsx<_> = open_workbook(path).map_err(|source| ReadError::Open {
        path: path.display().to_string(),
        source,
    })?;
    let names = workbook.sheet_names().to_owned();
    let mut sheets = Vec::with_capacity(names.len());
    for name in names {
        let range = workbook
            .worksheet_range(&name)
            .map_err(|source| ReadError::Sheet {
                name: name.clone(),
                source,
            })?;
        sheets.push((name, range));
    }
    Ok(sheets)
}
