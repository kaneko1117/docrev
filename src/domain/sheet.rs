use std::collections::HashMap;

use super::cell::CellValue;

/// A solid cell background from the workbook, sRGB.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FillColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
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
    col_widths: Vec<Option<u16>>,
    merges: Vec<MergedRange>,
    /// Solid cell backgrounds from the workbook, keyed by (row, col).
    fills: HashMap<(usize, usize), FillColor>,
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
        }
    }

    pub fn with_fills(mut self, fills: HashMap<(usize, usize), FillColor>) -> Self {
        self.fills = fills;
        self
    }

    pub fn fill_at(&self, row: usize, col: usize) -> Option<FillColor> {
        self.fills.get(&(row, col)).copied()
    }

    pub fn with_col_widths(mut self, widths: Vec<Option<u16>>) -> Self {
        self.col_widths = widths;
        self
    }

    pub fn with_merges(mut self, merges: Vec<MergedRange>) -> Self {
        self.merges = merges;
        self
    }

    pub fn col_width(&self, col: usize) -> Option<u16> {
        self.col_widths.get(col).copied().flatten()
    }

    pub fn merge_at(&self, row: usize, col: usize) -> Option<&MergedRange> {
        self.merges.iter().find(|m| m.contains(row, col))
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
}
