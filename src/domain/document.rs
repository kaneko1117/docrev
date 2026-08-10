use super::sheet::Sheet;

/// An ordered collection of sheets.
#[derive(Debug, Clone)]
pub struct Document {
    sheets: Vec<Sheet>,
}

impl Document {
    pub fn new(sheets: Vec<Sheet>) -> Self {
        Self { sheets }
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
