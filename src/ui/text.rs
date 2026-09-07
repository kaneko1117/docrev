use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::domain::cell::CellValue;
use crate::domain::sheet::{Alignment, Horizontal, Vertical};

pub(crate) fn cell_text(cell: &CellValue) -> String {
    sanitize(&cell.display_text())
}

/// One line per `\n`, `\r\n` or lone `\r`, each wrapped to `width`.
pub(crate) fn cell_lines(cell: &CellValue, width: usize) -> Vec<String> {
    cell.display_text()
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .split('\n')
        .flat_map(|line| wrap(&sanitize(line), width))
        .collect()
}

/// Callers that want line breaks honored split on them first.
pub(crate) fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// A char wider than `width` still gets a line of its own so the loop makes progress; empty text is
/// one empty line.
pub(crate) fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w > width && !line.is_empty() {
            lines.push(std::mem::take(&mut line));
            used = 0;
        }
        line.push(ch);
        used += w;
    }
    lines.push(line);
    lines
}

/// Cells of leading space per indent level.
const INDENT_WIDTH: usize = 2;

/// Where a cell's lines sit in a slot once the value type has filled in what the workbook left open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Placement {
    pub horizontal: Horizontal,
    pub vertical: Vertical,
    /// Cells, already capped so at least two cells of text remain (one wide character).
    pub indent: usize,
}

impl Placement {
    /// `numeric` values default to the right; indent counts only on left-aligned cells.
    pub(crate) fn resolve(alignment: Option<Alignment>, numeric: bool, width: usize) -> Self {
        let alignment = alignment.unwrap_or_default();
        let horizontal = alignment.horizontal.unwrap_or(if numeric {
            Horizontal::Right
        } else {
            Horizontal::Left
        });
        let indent = if horizontal == Horizontal::Left {
            (alignment.indent as usize * INDENT_WIDTH).min(width.saturating_sub(2))
        } else {
            0
        };
        Self {
            horizontal,
            vertical: alignment.vertical.unwrap_or(Vertical::Top),
            indent,
        }
    }

    /// Width left for the text once the indent is taken.
    pub(crate) fn text_width(&self, width: usize) -> usize {
        width.saturating_sub(self.indent)
    }

    /// One slot line of exactly `width` cells; overlong text is clipped, and a slot narrower
    /// than the indent shows only spaces.
    pub(crate) fn line(&self, text: &str, width: usize) -> String {
        let indent = self.indent.min(width);
        let inner = width - indent;
        let clipped = clip(text, inner);
        let aligned = match self.horizontal {
            Horizontal::Left => pad_right(&clipped, inner),
            Horizontal::Center => center(&clipped, inner),
            Horizontal::Right => pad_left(&clipped, inner),
        };
        format!("{}{aligned}", " ".repeat(indent))
    }

    /// Index of the slot line the first text line lands on when `lines` lines sit in `height`.
    pub(crate) fn offset(&self, lines: usize, height: usize) -> usize {
        let spare = height.saturating_sub(lines);
        match self.vertical {
            Vertical::Top => 0,
            Vertical::Center => spare / 2,
            Vertical::Bottom => spare,
        }
    }
}

pub(crate) fn clip(text: &str, max: usize) -> String {
    if text.width() <= max {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w > max.saturating_sub(1) {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    out
}

pub(crate) fn pad_right(text: &str, width: usize) -> String {
    format!("{text}{}", " ".repeat(width.saturating_sub(text.width())))
}

pub(crate) fn pad_left(text: &str, width: usize) -> String {
    format!("{}{text}", " ".repeat(width.saturating_sub(text.width())))
}

pub(crate) fn center(text: &str, width: usize) -> String {
    let pad = width.saturating_sub(text.width());
    let left = pad / 2;
    format!("{}{text}{}", " ".repeat(left), " ".repeat(pad - left))
}

/// Clipped from the front; the cursor block is always kept.
pub(crate) fn query_line(query: &str, width: usize) -> String {
    let mut out = String::from("█");
    let mut used = 1;
    for ch in query.chars().rev() {
        let w = ch.width().unwrap_or(0);
        if used + w > width {
            break;
        }
        out.insert(0, ch);
        used += w;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clips_by_display_width() {
        let long = "これはとても長い備考でありグリッド表示では切り詰められるはずの文字列";
        let clipped = clip(long, 24);
        assert!(clipped.ends_with('…'));
        assert!(clipped.width() <= 24);
        assert_eq!(clip("short", 24), "short");
    }

    #[test]
    fn sanitizes_control_chars() {
        assert_eq!(sanitize("a\nb\tc\r\n"), "a b c  ");
    }

    #[test]
    fn the_query_clips_from_the_front_keeping_the_cursor() {
        assert_eq!(query_line("abc", 10), "abc█");
        assert_eq!(query_line("", 10), "█");
        assert_eq!(query_line("abcdefgh", 5), "efgh█", "the tail stays");
        assert_eq!(query_line("あいうえお", 5), "えお█");
        assert_eq!(query_line("abc", 0), "█", "never empty, renderer clips");
    }

    #[test]
    fn cell_lines_break_where_the_author_did_then_wrap() {
        let cell = CellValue::Text("行1\n行2が長い\n行3".into());
        assert_eq!(cell_lines(&cell, 8), vec!["行1", "行2が長", "い", "行3"]);
        assert_eq!(
            cell_lines(&CellValue::Text("a\r\nb\rc".into()), 8),
            vec!["a", "b", "c"]
        );
        assert_eq!(cell_lines(&CellValue::Text("a\tb".into()), 8), vec!["a b"]);
        assert_eq!(cell_lines(&CellValue::Empty, 8), vec![""]);
    }

    #[test]
    fn placement_fills_in_the_value_type_and_caps_the_indent() {
        let none = Placement::resolve(None, false, 10);
        assert_eq!(
            (none.horizontal, none.vertical, none.indent),
            (Horizontal::Left, Vertical::Top, 0)
        );
        assert_eq!(
            Placement::resolve(None, true, 10).horizontal,
            Horizontal::Right
        );
        let explicit = Placement::resolve(
            Some(Alignment {
                horizontal: Some(Horizontal::Left),
                vertical: Some(Vertical::Bottom),
                indent: 2,
            }),
            true,
            10,
        );
        assert_eq!(
            (explicit.horizontal, explicit.vertical, explicit.indent),
            (Horizontal::Left, Vertical::Bottom, 4)
        );
        let deep = Placement::resolve(
            Some(Alignment {
                indent: 9,
                ..Alignment::default()
            }),
            false,
            6,
        );
        assert_eq!(deep.indent, 4, "two cells of text always remain");
        assert_eq!(Placement::resolve(None, false, 1).indent, 0);
        let right = Placement::resolve(
            Some(Alignment {
                horizontal: Some(Horizontal::Right),
                indent: 3,
                ..Alignment::default()
            }),
            false,
            10,
        );
        assert_eq!(right.indent, 0, "indent counts only on the left");
    }

    #[test]
    fn placement_lines_are_exactly_the_slot_width() {
        let left = Placement::resolve(
            Some(Alignment {
                indent: 1,
                ..Alignment::default()
            }),
            false,
            8,
        );
        assert_eq!(left.line("ab", 8), "  ab    ");
        assert_eq!(left.line("abcdefghij", 8), "  abcde…");
        let center = Placement::resolve(
            Some(Alignment {
                horizontal: Some(Horizontal::Center),
                ..Alignment::default()
            }),
            false,
            7,
        );
        assert_eq!(center.line("ab", 7), "  ab   ");
        assert_eq!(center.line("日本", 7), " 日本  ");
        assert_eq!(Placement::resolve(None, true, 5).line("42", 5), "   42");
        assert_eq!(left.line("", 1), " ", "narrower than the indent");
    }

    #[test]
    fn placement_offset_follows_the_vertical_choice() {
        let with = |vertical| {
            Placement::resolve(
                Some(Alignment {
                    vertical: Some(vertical),
                    ..Alignment::default()
                }),
                false,
                4,
            )
        };
        assert_eq!(with(Vertical::Top).offset(1, 4), 0);
        assert_eq!(with(Vertical::Center).offset(1, 4), 1);
        assert_eq!(with(Vertical::Bottom).offset(1, 4), 3);
        assert_eq!(
            with(Vertical::Bottom).offset(5, 4),
            0,
            "taller than the slot"
        );
    }

    #[test]
    fn wraps_by_display_width() {
        assert_eq!(wrap("abcdef", 4), vec!["abcd", "ef"]);
        // CJK chars are 2 cells wide: 4 cells fit two of them
        assert_eq!(wrap("あいうえお", 4), vec!["あい", "うえ", "お"]);
        assert_eq!(wrap("いあ", 3), vec!["い", "あ"]);
        assert_eq!(wrap("いいあ", 3), vec!["い", "い", "あ"]);
        assert_eq!(wrap("", 4), vec![""]);
        assert_eq!(wrap("abc", 0), vec![""], "zero width cannot loop");
        assert_eq!(wrap("あ", 1), vec!["あ"], "wider than the column");
    }
}
