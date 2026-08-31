use std::collections::HashMap;

use super::cell::CellValue;
use super::workbook_comment::WorkbookComment;

/// A color inherited from the workbook (cell background or font), sRGB.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// A color the workbook names rather than numbers. The presentation layer
/// is free to reinterpret it — the terminal theme hands these to the user's
/// own 16 colors, where a literal `Rgb` has nothing left to interpret.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NamedColor {
    Red,
    Blue,
    Green,
    Yellow,
    Magenta,
    Cyan,
    Black,
    White,
}

/// The color a cell's text takes. Which of the two a cell carries is
/// resolved outside the model; what the model keeps is the distinction the
/// ui branches on — a named color it may reinterpret, or a literal one it
/// may only paint or drop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TextColor {
    Named(NamedColor),
    Literal(Rgb),
}

/// An inclusive rectangle of merged cells; the value lives at the anchor
/// (top-left).
#[derive(Debug, Clone, PartialEq)]
pub struct MergedRange {
    pub start_row: usize,
    pub start_col: usize,
    pub end_row: usize,
    pub end_col: usize,
}

impl MergedRange {
    pub fn contains(&self, row: usize, col: usize) -> bool {
        (self.start_row..=self.end_row).contains(&row)
            && (self.start_col..=self.end_col).contains(&col)
    }

    pub fn anchor(&self) -> (usize, usize) {
        (self.start_row, self.start_col)
    }
}

/// A grid of cells, 0-based, row-major.
#[derive(Debug, Clone)]
pub struct Sheet {
    name: String,
    rows: Vec<Vec<CellValue>>,
    /// Author-set column widths from the workbook, index = column.
    col_widths: Vec<Option<f64>>,
    merges: Vec<MergedRange>,
    /// Solid cell backgrounds from the workbook, keyed by (row, col).
    fills: HashMap<(usize, usize), Rgb>,
    /// The color each cell's text takes, already resolved to one per cell.
    text_colors: HashMap<(usize, usize), TextColor>,
    /// Frozen panes from the workbook: (rows, cols) pinned while scrolling.
    frozen: (usize, usize),
    /// Formulas by (row, col), without their leading `=`.
    formulas: HashMap<(usize, usize), String>,
    /// The workbook's own comments (notes and threaded), read-only.
    workbook_comments: Vec<WorkbookComment>,
}

impl Sheet {
    const EMPTY: CellValue = CellValue::Empty;

    pub fn new(name: impl Into<String>, rows: Vec<Vec<CellValue>>) -> Self {
        Self {
            name: name.into(),
            rows,
            col_widths: Vec::new(),
            merges: Vec::new(),
            fills: HashMap::new(),
            text_colors: HashMap::new(),
            frozen: (0, 0),
            formulas: HashMap::new(),
            workbook_comments: Vec::new(),
        }
    }

    pub fn with_fills(mut self, fills: HashMap<(usize, usize), Rgb>) -> Self {
        self.fills = fills;
        self
    }

    pub fn fill_at(&self, row: usize, col: usize) -> Option<Rgb> {
        self.fills.get(&(row, col)).copied()
    }

    /// Text colors already resolved to one per cell: which source wins is
    /// the workbook format's rule, settled before the model sees it.
    pub fn with_text_colors(mut self, colors: HashMap<(usize, usize), TextColor>) -> Self {
        self.text_colors = colors;
        self
    }

    /// Widths as the file states them: fractional character counts.
    /// Rounding and clamping into terminal cells is the ui's decision.
    pub fn with_col_widths(mut self, widths: Vec<Option<f64>>) -> Self {
        self.col_widths = widths;
        self
    }

    pub fn with_merges(mut self, merges: Vec<MergedRange>) -> Self {
        self.merges = merges;
        self
    }

    /// Frozen panes: the workbook pins the first `rows`/`cols` while the
    /// rest scrolls.
    pub fn with_frozen(mut self, rows: usize, cols: usize) -> Self {
        self.frozen = (rows, cols);
        self
    }

    /// A formula cell without a cached result (files written by tools that
    /// never evaluate, e.g. openpyxl) lies outside the value grid — the grid
    /// grows to reach it, showing `Empty` where no result is known.
    pub fn with_formulas(mut self, formulas: HashMap<(usize, usize), String>) -> Self {
        for &(row, col) in formulas.keys() {
            if self.rows.len() <= row {
                self.rows.resize_with(row + 1, Vec::new);
            }
            let cells = &mut self.rows[row];
            if cells.len() <= col {
                cells.resize(col + 1, CellValue::Empty);
            }
        }
        self.formulas = formulas;
        self
    }

    /// The cell's own formula, without its leading `=`. Callers resolve
    /// merged regions to their anchor themselves, like `cell` vs
    /// `display_cell`.
    pub fn formula_at(&self, row: usize, col: usize) -> Option<&str> {
        self.formulas.get(&(row, col)).map(String::as_str)
    }

    /// Like formulas, a comment can sit outside the value grid (a note on
    /// an empty cell past the used range) — the grid grows to reach it, or
    /// its corner tint would be invisible and the cursor could never get
    /// there.
    pub fn with_workbook_comments(mut self, comments: Vec<WorkbookComment>) -> Self {
        for comment in &comments {
            if self.rows.len() <= comment.row {
                self.rows.resize_with(comment.row + 1, Vec::new);
            }
            let cells = &mut self.rows[comment.row];
            if cells.len() <= comment.col {
                cells.resize(comment.col + 1, CellValue::Empty);
            }
        }
        self.workbook_comments = comments;
        self
    }

    pub fn workbook_comments(&self) -> &[WorkbookComment] {
        &self.workbook_comments
    }

    /// The workbook comments a cursor on (row, col) should surface — inside
    /// a merged region the whole region counts as one cell, like threads.
    pub fn workbook_comments_at(&self, row: usize, col: usize) -> Vec<&WorkbookComment> {
        let merge = self.merge_at(row, col);
        let in_region = |r: usize, c: usize| match merge {
            Some(m) => m.contains(r, c),
            None => (r, c) == (row, col),
        };
        self.workbook_comments
            .iter()
            .filter(|comment| in_region(comment.row, comment.col))
            .collect()
    }

    pub fn frozen_rows(&self) -> usize {
        self.frozen.0
    }

    pub fn frozen_cols(&self) -> usize {
        self.frozen.1
    }

    pub fn col_width(&self, col: usize) -> Option<f64> {
        self.col_widths.get(col).copied().flatten()
    }

    /// The value a cell displays: inside a merged region that is the
    /// region's anchor (top-left) value, which is where the workbook keeps it.
    pub fn display_cell(&self, row: usize, col: usize) -> &CellValue {
        match self.merge_at(row, col) {
            Some(merge) => {
                let (anchor_row, anchor_col) = merge.anchor();
                self.cell(anchor_row, anchor_col)
            }
            None => self.cell(row, col),
        }
    }

    /// The fill a cell paints with: inside a merged region the anchor's fill
    /// covers the whole region.
    pub fn display_fill_at(&self, row: usize, col: usize) -> Option<Rgb> {
        let (row, col) = match self.merge_at(row, col) {
            Some(merge) => merge.anchor(),
            None => (row, col),
        };
        self.fill_at(row, col)
    }

    /// The color a cell's text takes: inside a merged region the anchor's
    /// styling applies to the whole region.
    pub fn text_color_at(&self, row: usize, col: usize) -> Option<TextColor> {
        let (row, col) = match self.merge_at(row, col) {
            Some(merge) => merge.anchor(),
            None => (row, col),
        };
        self.text_colors.get(&(row, col)).copied()
    }

    pub fn merge_at(&self, row: usize, col: usize) -> Option<&MergedRange> {
        self.merges.iter().find(|m| m.contains(row, col))
    }

    /// All merged regions — for whole-sheet scans that cannot afford a
    /// `merge_at` lookup per cell.
    pub fn merges(&self) -> &[MergedRange] {
        &self.merges
    }

    /// Cells actually stored in a row; cells beyond are `Empty`.
    pub fn row_len(&self, row: usize) -> usize {
        self.rows.get(row).map(Vec::len).unwrap_or(0)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn col_count(&self) -> usize {
        self.rows.iter().map(Vec::len).max().unwrap_or(0)
    }

    /// `Empty` outside the used range.
    pub fn cell(&self, row: usize, col: usize) -> &CellValue {
        self.rows
            .get(row)
            .and_then(|r| r.get(col))
            .unwrap_or(&Self::EMPTY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_of_range_is_empty() {
        let sheet = Sheet::new("s", vec![vec![CellValue::Number(1.0)]]);
        assert_eq!(sheet.cell(0, 0), &CellValue::Number(1.0));
        assert_eq!(sheet.cell(5, 5), &CellValue::Empty);
    }

    #[test]
    fn col_count_is_widest_row() {
        let sheet = Sheet::new(
            "s",
            vec![vec![], vec![CellValue::Empty, CellValue::Bool(true)]],
        );
        assert_eq!(sheet.row_count(), 2);
        assert_eq!(sheet.col_count(), 2);
    }

    fn merged_sheet() -> Sheet {
        Sheet::new(
            "s",
            vec![
                vec![
                    CellValue::Text("title".into()),
                    CellValue::Empty,
                    CellValue::Empty,
                ],
                vec![CellValue::Text("plain".into())],
            ],
        )
        .with_merges(vec![MergedRange {
            start_row: 0,
            start_col: 0,
            end_row: 0,
            end_col: 2,
        }])
    }

    #[test]
    fn display_cell_resolves_a_merge_to_its_anchor() {
        let sheet = merged_sheet();
        assert_eq!(sheet.cell(0, 2), &CellValue::Empty, "the raw cell is empty");
        assert_eq!(
            sheet.display_cell(0, 2),
            &CellValue::Text("title".into()),
            "but it displays the region's anchor value"
        );
        assert_eq!(
            sheet.display_cell(1, 0),
            &CellValue::Text("plain".into()),
            "cells outside a merge are unaffected"
        );
    }

    #[test]
    fn both_kinds_of_text_color_are_kept_apart() {
        let blue = Rgb { r: 0, g: 0, b: 255 };
        let sheet = Sheet::new("s", vec![vec![CellValue::Text("x".into()); 2]]).with_text_colors(
            HashMap::from([
                ((0, 0), TextColor::Named(NamedColor::Red)),
                ((0, 1), TextColor::Literal(blue)),
            ]),
        );

        assert_eq!(
            sheet.text_color_at(0, 0),
            Some(TextColor::Named(NamedColor::Red))
        );
        assert_eq!(sheet.text_color_at(0, 1), Some(TextColor::Literal(blue)));
    }

    #[test]
    fn a_merged_region_takes_its_anchor_styling() {
        let red = Rgb { r: 255, g: 0, b: 0 };
        let sheet = merged_sheet()
            .with_text_colors(HashMap::from([((0, 0), TextColor::Literal(red))]))
            .with_fills(HashMap::from([((0, 0), red)]));
        assert_eq!(sheet.text_color_at(0, 2), Some(TextColor::Literal(red)));
        assert_eq!(sheet.display_fill_at(0, 2), Some(red));
        assert_eq!(sheet.fill_at(0, 2), None, "the raw lookup stays raw");
    }

    #[test]
    fn cells_without_styling_have_no_color() {
        let sheet = Sheet::new("s", vec![vec![CellValue::Number(1.0)]]);
        assert_eq!(sheet.text_color_at(0, 0), None);
    }
}
