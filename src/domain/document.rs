use super::sheet::Sheet;

#[derive(Debug, Clone)]
pub struct Document {
    sheets: Vec<Sheet>,
}

impl Document {
    /// At least one sheet is shown: a workbook that hides every sheet opens with all of them.
    pub fn new(sheets: Vec<Sheet>) -> Self {
        let sheets = if !sheets.is_empty() && sheets.iter().all(Sheet::is_hidden) {
            sheets.into_iter().map(|s| s.with_hidden(false)).collect()
        } else {
            sheets
        };
        Self { sheets }
    }

    /// Index of the first sheet that is not hidden.
    pub fn first_shown(&self) -> Option<usize> {
        self.sheets.iter().position(|s| !s.is_hidden())
    }

    pub fn sheets(&self) -> &[Sheet] {
        &self.sheets
    }

    pub fn into_sheets(self) -> Vec<Sheet> {
        self.sheets
    }

    pub fn sheet_names(&self) -> impl Iterator<Item = &str> {
        self.sheets.iter().map(Sheet::name)
    }

    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.sheets.iter().position(|s| s.name() == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::cell::CellValue;

    fn sheet(name: &str, hidden: bool) -> Sheet {
        Sheet::new(name, vec![vec![CellValue::Empty]]).with_hidden(hidden)
    }

    #[test]
    fn an_all_hidden_workbook_shows_every_sheet() {
        let document = Document::new(vec![sheet("a", true), sheet("b", true)]);
        assert!(document.sheets().iter().all(|s| !s.is_hidden()));
        assert_eq!(document.first_shown(), Some(0));
    }

    #[test]
    fn a_partly_hidden_workbook_keeps_its_flags() {
        let document = Document::new(vec![sheet("a", true), sheet("b", false)]);
        assert!(document.sheets()[0].is_hidden());
        assert_eq!(document.first_shown(), Some(1));
        assert_eq!(Document::new(Vec::new()).first_shown(), None);
    }
}
