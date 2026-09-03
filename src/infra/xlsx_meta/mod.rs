//! Workbook metadata calamine does not expose, read straight from the archive.

use std::collections::HashMap;
use std::path::Path;

use thiserror::Error;

mod archive;
mod comments;
mod styles;
#[cfg(test)]
mod tests;
mod theme;
mod worksheet;

pub use comments::{RawWorkbookComment, workbook_comments};
pub use styles::{CellStyle, WorkbookStyles};
pub use worksheet::ColumnRange;

use archive::{entry_path, open_archive, parse_rel_targets, parse_sheet_ids, read_entry};
use styles::{parse_cell_styles, parse_styles};
use theme::{default_palette, parse_theme_palette};
use worksheet::{parse_cols, parse_hidden_rows, parse_pane};

#[derive(Debug, Error)]
#[error("{0}")]
pub struct MetaError(String);

/// Each attribute falls back to its default when its part fails to parse.
#[derive(Debug, Default)]
pub struct WorkbookMeta {
    pub cols: HashMap<String, Vec<ColumnRange>>,
    /// 0-based row indexes per sheet.
    pub hidden_rows: HashMap<String, Vec<u32>>,
    pub styles: WorkbookStyles,
    pub frozen: HashMap<String, (usize, usize)>,
}

/// Never fails: every attribute degrades to its default.
pub fn read_meta(document: &Path) -> WorkbookMeta {
    let Ok(mut archive) = open_archive(document) else {
        return WorkbookMeta::default();
    };

    // no theme part: default palette; unparsable theme: empty palette, so
    // theme-indexed colors drop rather than recolor
    let palette = match read_entry(&mut archive, "xl/theme/theme1.xml") {
        Ok(xml) => parse_theme_palette(&xml).unwrap_or_default(),
        Err(_) => default_palette(),
    };
    let styles = read_entry(&mut archive, "xl/styles.xml")
        .ok()
        .and_then(|xml| parse_styles(&xml, &palette).ok())
        .unwrap_or_default();
    let styles_active = !styles.iter().all(CellStyle::is_plain);

    let Ok(workbook) = read_entry(&mut archive, "xl/workbook.xml") else {
        return WorkbookMeta::default();
    };
    let Ok(rels) = read_entry(&mut archive, "xl/_rels/workbook.xml.rels") else {
        return WorkbookMeta::default();
    };
    let Ok(sheets) = parse_sheet_ids(&workbook) else {
        return WorkbookMeta::default();
    };
    let Ok(targets) = parse_rel_targets(&rels) else {
        return WorkbookMeta::default();
    };

    let mut meta = WorkbookMeta::default();
    let mut style_cells = HashMap::new();
    for (name, rid) in sheets {
        let Some(target) = targets.get(&rid) else {
            continue;
        };
        let Ok(xml) = read_entry(&mut archive, &entry_path(target)) else {
            continue;
        };
        // attributes default independently
        if let Ok(cols) = parse_cols(&xml)
            && !cols.is_empty()
        {
            meta.cols.insert(name.clone(), cols);
        }
        if let Ok(rows) = parse_hidden_rows(&xml)
            && !rows.is_empty()
        {
            meta.hidden_rows.insert(name.clone(), rows);
        }
        if let Ok(Some(frozen)) = parse_pane(&xml) {
            meta.frozen.insert(name.clone(), frozen);
        }
        if styles_active
            && let Ok(cells) = parse_cell_styles(&xml, &styles)
            && !cells.is_empty()
        {
            style_cells.insert(name, cells);
        }
    }
    if styles_active {
        meta.styles = WorkbookStyles {
            styles,
            sheets: style_cells,
        };
    }
    meta
}
