use std::collections::{HashMap, HashSet};

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
    /// The sheet itself is hidden in the workbook.
    hidden: bool,
    hidden_rows: HashSet<usize>,
    hidden_cols: HashSet<usize>,
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
            hidden: false,
            hidden_rows: HashSet::new(),
            hidden_cols: HashSet::new(),
        }
    }

    pub fn with_hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }

    pub fn with_hidden_rows(mut self, rows: HashSet<usize>) -> Self {
        self.hidden_rows = rows;
        self
    }

    pub fn with_hidden_cols(mut self, cols: HashSet<usize>) -> Self {
        self.hidden_cols = cols;
        self
    }

    pub fn is_hidden(&self) -> bool {
        self.hidden
    }

    pub fn row_hidden(&self, row: usize) -> bool {
        self.hidden_rows.contains(&row)
    }

    pub fn col_hidden(&self, col: usize) -> bool {
        self.hidden_cols.contains(&col)
    }

    pub fn cell_hidden(&self, row: usize, col: usize) -> bool {
        self.row_hidden(row) || self.col_hidden(col)
    }

    /// Nothing of the cell is on screen: inside a merged region, only when the whole region is
    /// hidden; the region's anchor may itself sit on a hidden row or column.
    pub fn anchor_hidden(&self, row: usize, col: usize) -> bool {
        self.shown_cell_of(row, col).is_none()
    }

    /// The first shown cell of the merged region around (row, col), or the cell itself when shown.
    pub fn shown_cell_of(&self, row: usize, col: usize) -> Option<(usize, usize)> {
        match self.merge_at(row, col) {
            Some(merge) => {
                let r = (merge.start_row..=merge.end_row).find(|&r| !self.row_hidden(r))?;
                let c = (merge.start_col..=merge.end_col).find(|&c| !self.col_hidden(c))?;
                Some((r, c))
            }
            None => (!self.cell_hidden(row, col)).then_some((row, col)),
        }
    }

    pub fn visible_rows(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.row_count()).filter(|&r| !self.row_hidden(r))
    }

    pub fn visible_cols(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.col_count()).filter(|&c| !self.col_hidden(c))
    }

    /// The nearest visible row at or after `row`, else at or before it; `None` when every row is hidden.
    pub fn nearest_visible_row(&self, row: usize) -> Option<usize> {
        let count = self.row_count();
        (row..count)
            .find(|&r| !self.row_hidden(r))
            .or_else(|| (0..row.min(count)).rev().find(|&r| !self.row_hidden(r)))
    }

    /// Like `nearest_visible_row`, for columns.
    pub fn nearest_visible_col(&self, col: usize) -> Option<usize> {
        let count = self.col_count();
        (col..count)
            .find(|&c| !self.col_hidden(c))
            .or_else(|| (0..col.min(count)).rev().find(|&c| !self.col_hidden(c)))
    }

    /// `steps` visible rows away from `row` (negative = up), stopping at the last one reachable.
    pub fn step_visible_row(&self, row: usize, steps: isize) -> usize {
        let mut current = row;
        let mut remaining = steps.unsigned_abs();
        let count = self.row_count();
        while remaining > 0 {
            let next = if steps < 0 {
                (0..current).rev().find(|&r| !self.row_hidden(r))
            } else {
                (current + 1..count).find(|&r| !self.row_hidden(r))
            };
            match next {
                Some(r) => current = r,
                None => break,
            }
            remaining -= 1;
        }
        current
    }

    /// Like `step_visible_row`, for columns.
    pub fn step_visible_col(&self, col: usize, steps: isize) -> usize {
        let mut current = col;
        let mut remaining = steps.unsigned_abs();
        let count = self.col_count();
        while remaining > 0 {
            let next = if steps < 0 {
                (0..current).rev().find(|&c| !self.col_hidden(c))
            } else {
                (current + 1..count).find(|&c| !self.col_hidden(c))
            };
            match next {
                Some(c) => current = c,
                None => break,
            }
            remaining -= 1;
        }
        current
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
    fn hidden_state_defaults_to_visible() {
        let sheet = Sheet::new("s", vec![vec![CellValue::Number(1.0); 3]; 3]);
        assert!(!sheet.is_hidden());
        assert!(!sheet.row_hidden(1) && !sheet.col_hidden(1));
        let sheet = sheet
            .with_hidden(true)
            .with_hidden_rows(HashSet::from([1]))
            .with_hidden_cols(HashSet::from([2]));
        assert!(sheet.is_hidden());
        assert!(sheet.row_hidden(1) && !sheet.row_hidden(0));
        assert!(sheet.col_hidden(2) && !sheet.col_hidden(1));
    }

    #[test]
    fn visible_navigation_skips_hidden_rows_and_cols() {
        let sheet = Sheet::new("s", vec![vec![CellValue::Number(1.0); 6]; 6])
            .with_hidden_rows(HashSet::from([1, 2, 5]))
            .with_hidden_cols(HashSet::from([0, 3]));
        assert_eq!(sheet.visible_rows().collect::<Vec<_>>(), vec![0, 3, 4]);
        assert_eq!(sheet.visible_cols().collect::<Vec<_>>(), vec![1, 2, 4, 5]);
        assert_eq!(sheet.nearest_visible_row(1), Some(3));
        assert_eq!(sheet.nearest_visible_row(5), Some(4), "falls back upward");
        assert_eq!(sheet.nearest_visible_col(0), Some(1));
        assert_eq!(sheet.step_visible_row(0, 1), 3);
        assert_eq!(sheet.step_visible_row(0, 2), 4);
        assert_eq!(sheet.step_visible_row(0, 9), 4, "stops at the last visible");
        assert_eq!(sheet.step_visible_row(4, -1), 3);
        assert_eq!(sheet.step_visible_row(3, -1), 0);
        assert_eq!(sheet.step_visible_row(0, -1), 0);
        assert_eq!(sheet.step_visible_col(2, 1), 4);
        assert_eq!(sheet.step_visible_col(4, -1), 2);
        let all_hidden =
            Sheet::new("s", vec![vec![CellValue::Empty]]).with_hidden_rows(HashSet::from([0]));
        assert_eq!(all_hidden.nearest_visible_row(0), None);
        assert!(sheet.cell_hidden(1, 1) && sheet.cell_hidden(0, 0) && !sheet.cell_hidden(0, 1));
    }

    #[test]
    fn a_merged_region_is_hidden_only_when_none_of_it_shows() {
        let merge = |sr, sc, er, ec| MergedRange {
            start_row: sr,
            start_col: sc,
            end_row: er,
            end_col: ec,
        };
        let sheet = Sheet::new("s", vec![vec![CellValue::Number(1.0); 4]; 4])
            .with_merges(vec![
                merge(0, 0, 2, 0),
                merge(0, 2, 0, 3),
                merge(3, 2, 3, 3),
            ])
            .with_hidden_rows(HashSet::from([0, 3]))
            .with_hidden_cols(HashSet::from([2]));
        // anchor row hidden, region shows on row 1
        assert!(!sheet.anchor_hidden(0, 0));
        assert_eq!(sheet.shown_cell_of(0, 0), Some((1, 0)));
        // anchor row hidden and the whole region is on that row
        assert!(sheet.anchor_hidden(0, 2));
        // region on a hidden row, whatever its columns
        assert!(sheet.anchor_hidden(3, 3));
        // plain cells
        assert!(sheet.anchor_hidden(1, 2) && !sheet.anchor_hidden(1, 1));
        assert_eq!(sheet.shown_cell_of(1, 1), Some((1, 1)));
    }

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
