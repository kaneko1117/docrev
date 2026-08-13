use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::domain::cell::CellValue;

pub(crate) fn cell_text(cell: &CellValue) -> String {
    match cell {
        CellValue::Empty => String::new(),
        CellValue::Text(s) | CellValue::DateTime(s) | CellValue::Error(s) => sanitize(s),
        CellValue::Number(n) => n.to_string(),
        CellValue::FormattedNumber { text, .. } => sanitize(text),
        CellValue::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
    }
}

/// Control characters would break the grid layout.
pub(crate) fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\n' | '\r' => '⏎',
            c if c.is_control() => ' ',
            c => c,
        })
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
        assert_eq!(sanitize("a\nb\tc"), "a⏎b c");
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
