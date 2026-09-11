use super::sheet::Sheet;
use super::text_document::TextDocument;

#[derive(Debug, Clone)]
pub enum Document {
    Workbook(Workbook),
    Text(TextDocument),
}

impl Document {
    pub fn from_sheets(sheets: Vec<Sheet>) -> Self {
        Document::Workbook(Workbook::new(sheets))
    }

    pub fn from_text(text: &str) -> Self {
        Document::Text(TextDocument::new(text))
    }

    /// `None` for a text document.
    pub fn workbook(&self) -> Option<&Workbook> {
        match self {
            Document::Workbook(workbook) => Some(workbook),
            Document::Text(_) => None,
        }
    }

    /// `None` for a text document.
    pub fn into_workbook(self) -> Option<Workbook> {
        match self {
            Document::Workbook(workbook) => Some(workbook),
            Document::Text(_) => None,
        }
    }

    /// `None` for a workbook.
    pub fn text(&self) -> Option<&TextDocument> {
        match self {
            Document::Workbook(_) => None,
            Document::Text(text) => Some(text),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Workbook {
    sheets: Vec<Sheet>,
}

impl Workbook {
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
        let document = Workbook::new(vec![sheet("a", true), sheet("b", true)]);
        assert!(document.sheets().iter().all(|s| !s.is_hidden()));
        assert_eq!(document.first_shown(), Some(0));
    }

    #[test]
    fn a_partly_hidden_workbook_keeps_its_flags() {
        let document = Workbook::new(vec![sheet("a", true), sheet("b", false)]);
        assert!(document.sheets()[0].is_hidden());
        assert_eq!(document.first_shown(), Some(1));
        assert_eq!(Workbook::new(Vec::new()).first_shown(), None);
    }

    #[test]
    fn each_kind_of_document_exposes_only_its_own_model() {
        let workbook = Document::from_sheets(vec![sheet("a", false)]);
        assert!(workbook.workbook().is_some());
        assert!(workbook.text().is_none());
        let text = Document::from_text("a\nb");
        assert!(text.workbook().is_none());
        assert_eq!(text.text().map(|t| t.len()), Some(2));
        assert!(text.into_workbook().is_none());
    }
}
