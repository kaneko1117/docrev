#[derive(Debug, Clone, PartialEq)]
pub enum CellValue {
    Empty,
    Text(String),
    Number(f64),
    Bool(bool),
    /// `YYYY-MM-DD HH:MM:SS`, converted in the adapter.
    DateTime(String),
    /// Excel error values such as `#DIV/0!`.
    Error(String),
}

impl CellValue {
    pub fn is_empty(&self) -> bool {
        matches!(self, CellValue::Empty)
    }
}
