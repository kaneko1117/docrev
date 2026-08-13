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
}
