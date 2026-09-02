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
    },
    Bool(bool),
    /// A date/time cell as the workbook displays it; `raw` keeps the
    /// machine-readable rendering (`2026-08-31 00:00:00`, time-only
    /// `13:05:00`, elapsed `36:00:00`) for agents.
    DateTime {
        text: String,
        raw: String,
    },
    /// Excel error values such as `#DIV/0!`.
    Error(String),
}

impl CellValue {
    pub fn is_empty(&self) -> bool {
        matches!(self, CellValue::Empty)
    }

    pub fn is_number(&self) -> bool {
        matches!(
            self,
            CellValue::Number(_) | CellValue::FormattedNumber { .. }
        )
    }

    pub fn is_datetime(&self) -> bool {
        matches!(self, CellValue::DateTime { .. })
    }

    /// How the workbook displays this value: a formatted number shows its
    /// format's output, a bare number its shortest round-tripping form.
    pub fn display_text(&self) -> String {
        match self {
            CellValue::Empty => String::new(),
            CellValue::Text(s) | CellValue::Error(s) => s.clone(),
            CellValue::DateTime { text, .. } => text.clone(),
            CellValue::Number(n) => n.to_string(),
            CellValue::FormattedNumber { text, .. } => text.clone(),
            CellValue::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_number_covers_formatted_numbers() {
        assert!(CellValue::Number(1.0).is_number());
        assert!(
            CellValue::FormattedNumber {
                value: 0.15,
                text: "15%".into(),
            }
            .is_number()
        );
        assert!(!CellValue::Text("1".into()).is_number());
        assert!(!CellValue::Empty.is_number());
    }

    #[test]
    fn display_text_follows_general_rules() {
        assert_eq!(CellValue::Empty.display_text(), "");
        assert_eq!(CellValue::Number(120.0).display_text(), "120");
        assert_eq!(CellValue::Number(80.5).display_text(), "80.5");
        assert_eq!(CellValue::Bool(true).display_text(), "TRUE");
        assert_eq!(CellValue::Bool(false).display_text(), "FALSE");
        assert_eq!(CellValue::Text("あ".into()).display_text(), "あ");
        assert_eq!(
            CellValue::FormattedNumber {
                value: 0.15,
                text: "15%".into(),
            }
            .display_text(),
            "15%",
            "the format's output wins over the raw value"
        );
    }
}
