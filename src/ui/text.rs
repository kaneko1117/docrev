use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::domain::cell::CellValue;

/// The workbook's display text, made safe for a terminal line.
pub(crate) fn cell_text(cell: &CellValue) -> String {
    sanitize(&cell.display_text())
}

/// The workbook's display text as grid lines: one per line break the
/// author typed (Alt+Enter), each wrapped to `width` in turn. `\r\n` and a
/// lone `\r` count as one break, like Excel treats them.
pub(crate) fn cell_lines(cell: &CellValue, width: usize) -> Vec<String> {
    cell.display_text()
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .split('\n')
        .flat_map(|line| wrap(&sanitize(line), width))
        .collect()
}

/// Control characters would break a terminal line; callers that want
/// line breaks honored split on them first (see `cell_lines`).
pub(crate) fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Breaks text into display-width-sized lines, char by char (CJK-safe).
/// A char wider than `width` still gets a line of its own so the result
/// always makes progress; empty text is one empty line.
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

/// A query line with its cursor block, clipped from the front: when the
/// query outgrows the field, the end being typed is what must stay visible.
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
        // CJK: a char that would half-fit is dropped whole
        assert_eq!(query_line("あいうえお", 5), "えお█");
        assert_eq!(query_line("abc", 0), "█", "never empty, renderer clips");
    }

    #[test]
    fn cell_lines_break_where_the_author_did_then_wrap() {
        let cell = CellValue::Text("行1\n行2が長い\n行3".into());
        assert_eq!(cell_lines(&cell, 8), vec!["行1", "行2が長", "い", "行3"]);
        // CRLF and a bare CR are one break each, never a blank line
        assert_eq!(
            cell_lines(&CellValue::Text("a\r\nb\rc".into()), 8),
            vec!["a", "b", "c"]
        );
        // other control characters still become a space on the same line
        assert_eq!(cell_lines(&CellValue::Text("a\tb".into()), 8), vec!["a b"]);
        assert_eq!(cell_lines(&CellValue::Empty, 8), vec![""]);
    }

    #[test]
    fn wraps_by_display_width() {
        assert_eq!(wrap("abcdef", 4), vec!["abcd", "ef"]);
        // CJK chars are 2 cells wide: 4 cells fit two of them
        assert_eq!(wrap("あいうえお", 4), vec!["あい", "うえ", "お"]);
        // a wide char never splits: 「あ」(2 cells) does not fit next to
        // 「い」 in 3 cells, so it moves whole to the next line
        assert_eq!(wrap("いあ", 3), vec!["い", "あ"]);
        assert_eq!(wrap("いいあ", 3), vec!["い", "い", "あ"]);
        assert_eq!(wrap("", 4), vec![""]);
        assert_eq!(wrap("abc", 0), vec![""], "zero width cannot loop");
        assert_eq!(wrap("あ", 1), vec!["あ"], "wider than the column");
    }
}
