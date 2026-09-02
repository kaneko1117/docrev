//! What calamine does not expose, read straight from the archive: column
//! widths, frozen panes, cell styles (number formats, fills, fonts) and the
//! workbook's own comments. All of it is supplementary, so none of it may
//! block opening: `read_meta` degrades each attribute to its default here,
//! while `workbook_comments` reports its error and leaves the adapter to
//! decide.

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
pub use worksheet::ColumnWidth;

use archive::{entry_path, open_archive, parse_rel_targets, parse_sheet_ids, read_entry};
use styles::{parse_cell_styles, parse_styles};
use theme::{default_palette, parse_theme_palette};
use worksheet::{parse_cols, parse_pane};

#[derive(Debug, Error)]
#[error("{0}")]
pub struct MetaError(String);

/// Everything cosmetic read from the archive in one pass: widths, styles
/// and frozen panes. Each attribute falls back to its default when its
/// part fails to parse — a cosmetic failure must never block opening.
#[derive(Debug, Default)]
pub struct WorkbookMeta {
    pub widths: HashMap<String, Vec<ColumnWidth>>,
    pub styles: WorkbookStyles,
    pub frozen: HashMap<String, (usize, usize)>,
}

/// Opens the archive once, resolves sheet name → part path once, and walks
/// each sheet XML once for every attribute. calamine exposes none of these.
pub fn read_meta(document: &Path) -> WorkbookMeta {
    let Ok(mut archive) = open_archive(document) else {
        return WorkbookMeta::default();
    };

    // a workbook without a theme part legitimately resolves against the
    // default Office palette; a theme that exists but cannot be parsed must
    // not silently recolor cells, so its theme-indexed fills are dropped
    // (empty palette) while rgb fills stay untouched
    let palette = match read_entry(&mut archive, "xl/theme/theme1.xml") {
        Ok(xml) => parse_theme_palette(&xml).unwrap_or_default(),
        Err(_) => default_palette(),
    };
    let styles = read_entry(&mut archive, "xl/styles.xml")
        .ok()
        .and_then(|xml| parse_styles(&xml, &palette).ok())
        .unwrap_or_default();
    // when no style resolves to anything visible, per-cell collection is
    // skipped entirely and the styles result stays default
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
        // attributes default independently: one sheet's broken <cols> must
        // strip neither its own pane nor the other sheets' widths
        if let Ok(cols) = parse_cols(&xml)
            && !cols.is_empty()
        {
            meta.widths.insert(name.clone(), cols);
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
