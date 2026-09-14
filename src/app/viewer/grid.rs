use crate::domain::document::Workbook;
use crate::domain::sheet::Sheet;

use super::super::error::DocumentError;

/// Non-empty by construction.
struct Sheets {
    first: Sheet,
    rest: Vec<Sheet>,
}

impl Sheets {
    fn get(&self, index: usize) -> &Sheet {
        match index.checked_sub(1) {
            None => &self.first,
            Some(i) => self.rest.get(i).unwrap_or(&self.first),
        }
    }

    fn len(&self) -> usize {
        1 + self.rest.len()
    }

    /// Indices of the sheets that are not hidden; never empty (see `Workbook::new`).
    fn shown(&self) -> Vec<usize> {
        let shown: Vec<usize> = (0..self.len())
            .filter(|&i| !self.get(i).is_hidden())
            .collect();
        if shown.is_empty() { vec![0] } else { shown }
    }

    fn first_shown(&self) -> usize {
        self.shown()[0]
    }
}

/// The sheets of a workbook with one cursor per sheet and the active one.
pub(super) struct Grid {
    sheets: Sheets,
    cursors: Vec<(usize, usize)>,
    active: usize,
}

impl Grid {
    /// The active sheet is the first shown one; every cursor sits on the nearest visible cell to A1.
    pub(super) fn new(workbook: Workbook) -> Result<Self, DocumentError> {
        let mut sheets = workbook.into_sheets().into_iter();
        let Some(first) = sheets.next() else {
            return Err(DocumentError::EmptyDocument);
        };
        let rest: Vec<Sheet> = sheets.collect();
        let cursors = std::iter::once(&first)
            .chain(&rest)
            .map(|sheet| snap_visible(sheet, (0, 0)))
            .collect();
        let sheets = Sheets { first, rest };
        let active = sheets.first_shown();
        Ok(Self {
            sheets,
            cursors,
            active,
        })
    }

    pub(super) fn sheet(&self) -> &Sheet {
        self.sheets.get(self.active)
    }

    /// An index past the end yields the first sheet.
    pub(super) fn sheet_at(&self, index: usize) -> &Sheet {
        self.sheets.get(index)
    }

    pub(super) fn sheet_named(&self, name: &str) -> Option<&Sheet> {
        (0..self.len())
            .map(|i| self.sheet_at(i))
            .find(|s| s.name() == name)
    }

    pub(super) fn sheet_names(&self) -> Vec<&str> {
        std::iter::once(self.sheets.first.name())
            .chain(self.sheets.rest.iter().map(Sheet::name))
            .collect()
    }

    pub(super) fn len(&self) -> usize {
        self.sheets.len()
    }

    /// Sheet indices in the tab strip and picker: the ones the workbook does not hide.
    pub(super) fn shown(&self) -> Vec<usize> {
        self.sheets.shown()
    }

    pub(super) fn active(&self) -> usize {
        self.active
    }

    pub(super) fn set_active(&mut self, index: usize) {
        self.active = index;
    }

    pub(super) fn cursor(&self) -> (usize, usize) {
        self.cursors.get(self.active).copied().unwrap_or((0, 0))
    }

    pub(super) fn set_cursor(&mut self, cursor: (usize, usize)) {
        if let Some(slot) = self.cursors.get_mut(self.active) {
            *slot = cursor;
        }
    }

    /// The shown sheet `step` tabs away, wrapping around.
    pub(super) fn neighbour_sheet(&self, step: isize) -> usize {
        let shown = self.sheets.shown();
        let position = shown.iter().position(|&i| i == self.active).unwrap_or(0);
        let len = shown.len() as isize;
        shown[((position as isize + step).rem_euclid(len)) as usize]
    }

    /// Active sheet and cursors carry over by sheet name, clamped; a sheet that became hidden
    /// hands over to the first shown one.
    pub(super) fn replace(&mut self, workbook: Workbook) -> Result<(), DocumentError> {
        let new = Self::new(workbook)?;
        let old_names = self.sheet_names();
        let cursors: Vec<(usize, usize)> = (0..new.len())
            .map(|i| {
                let sheet = new.sheet_at(i);
                let (row, col) = old_names
                    .iter()
                    .position(|name| *name == sheet.name())
                    .and_then(|old| self.cursors.get(old).copied())
                    .unwrap_or((0, 0));
                let clamped = (
                    row.min(sheet.row_count().saturating_sub(1)),
                    col.min(sheet.col_count().saturating_sub(1)),
                );
                snap_visible(sheet, clamped)
            })
            .collect();
        let active_name = old_names.get(self.active).copied().unwrap_or_default();
        let active = (0..new.len())
            .position(|i| new.sheet_at(i).name() == active_name && !new.sheet_at(i).is_hidden())
            .unwrap_or_else(|| new.sheets.first_shown());
        self.sheets = new.sheets;
        self.cursors = cursors;
        self.active = active;
        Ok(())
    }
}

/// The nearest visible cell; an all-hidden axis keeps the given index.
fn snap_visible(sheet: &Sheet, (row, col): (usize, usize)) -> (usize, usize) {
    (
        sheet.nearest_visible_row(row).unwrap_or(row),
        sheet.nearest_visible_col(col).unwrap_or(col),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::domain::cell::CellValue;

    fn sheet(name: &str, rows: usize, cols: usize) -> Sheet {
        Sheet::new(name, vec![vec![CellValue::Number(1.0); cols]; rows])
    }

    fn grid(sheets: Vec<Sheet>) -> Grid {
        Grid::new(Workbook::new(sheets)).unwrap()
    }

    #[test]
    fn a_workbook_without_sheets_is_rejected() {
        let err = Grid::new(Workbook::new(Vec::new())).err();
        assert!(matches!(err, Some(DocumentError::EmptyDocument)), "{err:?}");
    }

    #[test]
    fn opens_on_the_first_shown_sheet_with_cursors_on_visible_cells() {
        let g = grid(vec![
            sheet("hidden", 2, 2).with_hidden(true),
            sheet("shown", 3, 3).with_hidden_rows(HashSet::from([0])),
        ]);
        assert_eq!(g.active(), 1);
        assert_eq!(g.sheet().name(), "shown");
        assert_eq!(g.cursor(), (1, 0), "row 0 is hidden");
        assert_eq!(g.len(), 2);
        assert_eq!(g.shown(), vec![1]);
        assert_eq!(g.sheet_names(), vec!["hidden", "shown"]);
        assert_eq!(
            g.sheet_at(9).name(),
            "hidden",
            "past the end: the first sheet"
        );
    }

    #[test]
    fn the_cursor_belongs_to_the_active_sheet() {
        let mut g = grid(vec![sheet("one", 3, 3), sheet("two", 3, 3)]);
        g.set_cursor((2, 1));
        assert_eq!(g.cursor(), (2, 1));
        g.set_active(1);
        assert_eq!(g.cursor(), (0, 0), "each sheet keeps its own cursor");
        g.set_active(0);
        assert_eq!(g.cursor(), (2, 1));
    }

    #[test]
    fn neighbour_sheet_skips_hidden_sheets_and_wraps() {
        let g = grid(vec![
            sheet("a", 1, 1),
            sheet("b", 1, 1).with_hidden(true),
            sheet("c", 1, 1),
        ]);
        assert_eq!(g.neighbour_sheet(1), 2, "b is hidden");
        assert_eq!(g.neighbour_sheet(-1), 2, "wraps backwards");
        assert_eq!(g.neighbour_sheet(2), 0, "wraps forwards");
    }

    #[test]
    fn replace_carries_cursors_over_by_name_and_clamps_them() {
        let mut g = grid(vec![sheet("one", 5, 5), sheet("two", 5, 5)]);
        g.set_cursor((4, 4));
        g.set_active(1);
        g.set_cursor((3, 3));
        g.replace(Workbook::new(vec![
            sheet("two", 2, 2),
            sheet("one", 5, 5).with_hidden_rows(HashSet::from([4])),
            sheet("new", 1, 1),
        ]))
        .unwrap();
        assert_eq!(g.active(), 0, "the active sheet follows its name");
        assert_eq!(g.cursor(), (1, 1), "clamped into the smaller sheet");
        g.set_active(1);
        assert_eq!(g.cursor(), (3, 4), "snapped off the row that became hidden");
        g.set_active(2);
        assert_eq!(g.cursor(), (0, 0), "a new sheet starts at A1");
    }

    #[test]
    fn replace_hands_a_sheet_that_became_hidden_over_to_the_first_shown_one() {
        let mut g = grid(vec![sheet("one", 1, 1), sheet("two", 1, 1)]);
        g.set_active(1);
        g.replace(Workbook::new(vec![
            sheet("one", 1, 1),
            sheet("two", 1, 1).with_hidden(true),
        ]))
        .unwrap();
        assert_eq!(g.active(), 0);
        g.replace(Workbook::new(vec![sheet("three", 1, 1)]))
            .unwrap();
        assert_eq!(
            g.active(),
            0,
            "a vanished name falls back to the first shown sheet"
        );
    }

    #[test]
    fn replace_with_no_sheets_fails_and_leaves_the_grid_untouched() {
        let mut g = grid(vec![sheet("one", 3, 3)]);
        g.set_cursor((2, 2));
        let err = g.replace(Workbook::new(Vec::new())).unwrap_err();
        assert!(matches!(err, DocumentError::EmptyDocument), "{err:?}");
        assert_eq!(g.sheet().name(), "one");
        assert_eq!(g.cursor(), (2, 2));
    }
}
