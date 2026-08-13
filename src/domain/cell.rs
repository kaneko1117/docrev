use super::number_format::FormatColor;

#[derive(Debug, Clone, PartialEq)]
pub enum CellValue {
    Empty,
    Text(String),
    Number(f64),
    /// A number whose workbook format produced a display string
    /// (e.g. `0.15` shown as `15%`); the raw value is kept for agents.
    FormattedNumber {
        value: f64,
        text: String,
        color: Option<FormatColor>,
    },
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
