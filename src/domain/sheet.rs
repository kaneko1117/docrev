use std::collections::HashMap;

use super::cell::CellValue;
use super::workbook_comment::WorkbookComment;

/// sRGB.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// A color the workbook names; the ui may reinterpret it, unlike `Rgb`.
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TextColor {
    Named(NamedColor),
    Literal(Rgb),
}

/// Inclusive; the value lives at the top-left anchor.
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

/// 0-based, row-major.
#[derive(Debug, Clone)]
pub struct Sheet {
    name: String,
    rows: Vec<Vec<CellValue>>,
    /// Index = column.
    col_widths: Vec<Option<f64>>,
    merges: Vec<MergedRange>,
    /// Keyed by (row, col).
    fills: HashMap<(usize, usize), Rgb>,
    text_colors: HashMap<(usize, usize), TextColor>,
    /// (rows, cols) pinned while scrolling.
    frozen: (usize, usize),
    /// By (row, col), without the leading `=`.
    formulas: HashMap<(usize, usize), String>,
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

    pub fn with_text_colors(mut self, colors: HashMap<(usize, usize), TextColor>) -> Self {
        self.text_colors = colors;
        self
    }

    /// Fractional character counts as the file states them; the ui rounds.
    pub fn with_col_widths(mut self, widths: Vec<Option<f64>>) -> Self {
        self.col_widths = widths;
        self
    }

    pub fn with_merges(mut self, merges: Vec<MergedRange>) -> Self {
        self.merges = merges;
        self
    }

    pub fn with_frozen(mut self, rows: usize, cols: usize) -> Self {
        self.frozen = (rows, cols);
        self
    }

    /// Grows the grid to reach a formula outside the value range (no cached result).
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

    /// Without the leading `=`; merged regions are not resolved to their anchor.
    pub fn formula_at(&self, row: usize, col: usize) -> Option<&str> {
        self.formulas.get(&(row, col)).map(String::as_str)
    }

    /// Grows the grid to reach a comment outside the value range.
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

    /// Inside a merged region the whole region counts as one cell.
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

    /// Inside a merged region, the anchor's value.
    pub fn display_cell(&self, row: usize, col: usize) -> &CellValue {
        match self.merge_at(row, col) {
            Some(merge) => {
                let (anchor_row, anchor_col) = merge.anchor();
                self.cell(anchor_row, anchor_col)
            }
            None => self.cell(row, col),
        }
    }

    /// Inside a merged region, the anchor's fill.
    pub fn display_fill_at(&self, row: usize, col: usize) -> Option<Rgb> {
        let (row, col) = match self.merge_at(row, col) {
            Some(merge) => merge.anchor(),
            None => (row, col),
        };
        self.fill_at(row, col)
    }

    /// Inside a merged region, the anchor's color.
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

    pub fn merges(&self) -> &[MergedRange] {
        &self.merges
    }

    /// Cells beyond are `Empty`.
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
