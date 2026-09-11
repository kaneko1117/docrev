/// 0-based; the 1-based forms (A1 notation, line numbers) are converted only by the methods here.
#[derive(Debug, Clone, PartialEq)]
pub enum Anchor {
    Cell { sheet: String, row: u32, col: u32 },
    Line { line: u32 },
}

impl Anchor {
    pub fn cell(sheet: impl Into<String>, row: u32, col: u32) -> Self {
        Anchor::Cell {
            sheet: sheet.into(),
            row,
            col,
        }
    }

    pub fn line(line: u32) -> Self {
        Anchor::Line { line }
    }

    /// `13` -> line 12; `None` for 0.
    pub fn from_line_number(number: u32) -> Option<Self> {
        number.checked_sub(1).map(Anchor::line)
    }

    /// `None` for a line anchor.
    pub fn sheet(&self) -> Option<&str> {
        match self {
            Anchor::Cell { sheet, .. } => Some(sheet),
            Anchor::Line { .. } => None,
        }
    }

    /// 1-based; `None` for a cell anchor.
    pub fn line_number(&self) -> Option<u32> {
        match self {
            Anchor::Cell { .. } => None,
            Anchor::Line { line } => Some(line.saturating_add(1)),
        }
    }

    /// `"B3"` or `"line 13"`: the place inside its sheet or file.
    pub fn position(&self) -> String {
        match self {
            Anchor::Cell { row, col, .. } => Self::a1(*row, *col),
            Anchor::Line { line } => format!("line {}", line.saturating_add(1)),
        }
    }

    /// `"売上!B3"` or `"line 13"`: the place inside the document.
    pub fn label(&self) -> String {
        match self {
            Anchor::Cell { sheet, .. } => format!("{sheet}!{}", self.position()),
            Anchor::Line { .. } => self.position(),
        }
    }

    /// `row 2, col 1` -> `"B3"`.
    pub fn a1(row: u32, col: u32) -> String {
        format!("{}{}", Self::column_label(col), row + 1)
    }

    /// `0 -> "A"`, `25 -> "Z"`, `26 -> "AA"`.
    pub fn column_label(index: u32) -> String {
        let mut index = index;
        let mut reversed = Vec::new();
        loop {
            reversed.push(char::from(b'A' + (index % 26) as u8));
            if index < 26 {
                break;
            }
            index = index / 26 - 1;
        }
        reversed.iter().rev().collect()
    }

    /// Splits on the last `!` (sheet names may contain `!`).
    pub fn parse_ref(reference: &str) -> Option<Anchor> {
        let (sheet, cell) = reference.rsplit_once('!')?;
        if sheet.is_empty() {
            return None;
        }
        let (row, col) = Self::parse_cell_ref(cell)?;
        Some(Anchor::cell(sheet, row, col))
    }

    /// `"B3"` -> `(row 2, col 1)`; `None` if malformed.
    pub fn parse_cell_ref(reference: &str) -> Option<(u32, u32)> {
        let letters: String = reference
            .chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .collect();
        let digits = reference.get(letters.len()..)?;
        if letters.is_empty() || digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let mut col: u32 = 0;
        for ch in letters.chars() {
            let value = ch.to_ascii_uppercase() as u32 - 'A' as u32 + 1;
            col = col.checked_mul(26)?.checked_add(value)?;
        }
        let row: u32 = digits.parse().ok()?;
        if row == 0 {
            return None;
        }
        Some((row - 1, col - 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_labels() {
        assert_eq!(Anchor::column_label(0), "A");
        assert_eq!(Anchor::column_label(25), "Z");
        assert_eq!(Anchor::column_label(26), "AA");
        assert_eq!(Anchor::column_label(701), "ZZ");
        assert_eq!(Anchor::column_label(702), "AAA");
    }

    #[test]
    fn a1_round_trips() {
        for (row, col) in [(0, 0), (2, 1), (9, 26), (99, 701)] {
            assert_eq!(
                Anchor::parse_cell_ref(&Anchor::a1(row, col)),
                Some((row, col))
            );
        }
    }

    #[test]
    fn line_numbers_are_one_based_at_the_edges() {
        assert_eq!(Anchor::from_line_number(13), Some(Anchor::line(12)));
        assert_eq!(Anchor::from_line_number(1), Some(Anchor::line(0)));
        assert_eq!(Anchor::from_line_number(0), None);
        assert_eq!(Anchor::line(12).line_number(), Some(13));
        assert_eq!(Anchor::cell("s", 0, 0).line_number(), None);
    }

    #[test]
    fn labels_name_the_place_in_each_kind_of_document() {
        let cell = Anchor::cell("売上", 2, 1);
        assert_eq!(cell.position(), "B3");
        assert_eq!(cell.label(), "売上!B3");
        assert_eq!(cell.sheet(), Some("売上"));
        let line = Anchor::line(12);
        assert_eq!(line.position(), "line 13");
        assert_eq!(line.label(), "line 13");
        assert_eq!(line.sheet(), None);
    }

    #[test]
    fn parses_common_references() {
        assert_eq!(Anchor::parse_cell_ref("B3"), Some((2, 1)));
        assert_eq!(Anchor::parse_cell_ref("b3"), Some((2, 1)));
        assert_eq!(Anchor::parse_cell_ref("AA10"), Some((9, 26)));
    }

    #[test]
    fn parses_full_references() {
        assert_eq!(
            Anchor::parse_ref("売上!B3"),
            Some(Anchor::cell("売上", 2, 1))
        );
        assert_eq!(
            Anchor::parse_ref("A!B!C3"),
            Some(Anchor::cell("A!B", 2, 2)),
            "splits on the last '!'"
        );
        for bad in ["B3", "!B3", "売上!", "売上!nope"] {
            assert_eq!(Anchor::parse_ref(bad), None, "should reject {bad:?}");
        }
    }

    #[test]
    fn rejects_malformed_references() {
        for bad in ["", "B", "3", "B0", "3B", "B3x", "B３"] {
            assert_eq!(Anchor::parse_cell_ref(bad), None, "should reject {bad:?}");
        }
    }
}
