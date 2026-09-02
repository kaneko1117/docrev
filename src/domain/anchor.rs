/// 0-based; A1 notation is converted only by the methods here.
#[derive(Debug, Clone, PartialEq)]
pub enum Anchor {
    Cell { sheet: String, row: u32, col: u32 },
}

impl Anchor {
    pub fn cell(sheet: impl Into<String>, row: u32, col: u32) -> Self {
        Anchor::Cell {
            sheet: sheet.into(),
            row,
            col,
        }
    }

    pub fn sheet(&self) -> &str {
        match self {
            Anchor::Cell { sheet, .. } => sheet,
        }
    }

    /// `row 2, col 1` -> `"B3"`.
    pub fn cell_ref(&self) -> String {
        match self {
            Anchor::Cell { row, col, .. } => format!("{}{}", Self::column_label(*col), row + 1),
        }
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
    fn cell_ref_round_trips() {
        for (row, col) in [(0, 0), (2, 1), (9, 26), (99, 701)] {
            let anchor = Anchor::cell("s", row, col);
            assert_eq!(Anchor::parse_cell_ref(&anchor.cell_ref()), Some((row, col)));
        }
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
