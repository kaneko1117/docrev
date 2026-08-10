use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::domain::cell::CellValue;
use crate::domain::sheet::Sheet;

const MAX_CELL_WIDTH: usize = 24;

pub fn render(sheet: &Sheet, position: usize, total: usize) -> String {
    let mut out = format!("Sheet: {} ({}/{})\n\n", sheet.name(), position + 1, total);
    let col_count = sheet.col_count();
    if sheet.row_count() == 0 || col_count == 0 {
        out.push_str("(empty sheet)\n");
        return out;
    }

    let texts: Vec<Vec<String>> = (0..sheet.row_count())
        .map(|r| {
            (0..col_count)
                .map(|c| clip(&cell_text(sheet.cell(r, c))))
                .collect()
        })
        .collect();

    let row_label_width = sheet.row_count().to_string().len();
    let col_widths: Vec<usize> = (0..col_count)
        .map(|c| {
            let body = texts
                .iter()
                .filter_map(|row| row.get(c))
                .map(|t| t.width())
                .max()
                .unwrap_or(0);
            column_label(c as u32).width().max(body)
        })
        .collect();

    let border = |l: char, m: char, r: char| {
        let mut line = String::new();
        line.push(l);
        line.push_str(&"─".repeat(row_label_width + 2));
        for w in &col_widths {
            line.push(m);
            line.push_str(&"─".repeat(w + 2));
        }
        line.push(r);
        line.push('\n');
        line
    };

    out.push_str(&border('┌', '┬', '┐'));

    out.push('│');
    out.push_str(&format!(" {} ", " ".repeat(row_label_width)));
    for (c, w) in col_widths.iter().enumerate() {
        out.push('│');
        out.push_str(&format!(" {} ", center(&column_label(c as u32), *w)));
    }
    out.push_str("│\n");

    out.push_str(&border('├', '┼', '┤'));

    for (r, row) in texts.iter().enumerate() {
        out.push('│');
        out.push_str(&format!(" {:>rw$} ", r + 1, rw = row_label_width));
        for (c, text) in row.iter().enumerate() {
            let w = col_widths.get(c).copied().unwrap_or(0);
            let aligned = if matches!(sheet.cell(r, c), CellValue::Number(_)) {
                pad_left(text, w)
            } else {
                pad_right(text, w)
            };
            out.push('│');
            out.push_str(&format!(" {aligned} "));
        }
        out.push_str("│\n");
    }

    out.push_str(&border('└', '┴', '┘'));
    out
}

fn cell_text(cell: &CellValue) -> String {
    match cell {
        CellValue::Empty => String::new(),
        CellValue::Text(s) | CellValue::DateTime(s) | CellValue::Error(s) => sanitize(s),
        CellValue::Number(n) => n.to_string(),
        CellValue::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
    }
}

/// Control characters would break the table layout.
fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\n' | '\r' => '⏎',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect()
}

fn column_label(index: u32) -> String {
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

fn clip(text: &str) -> String {
    if text.width() <= MAX_CELL_WIDTH {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w > MAX_CELL_WIDTH - 1 {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    out
}

fn pad_right(text: &str, width: usize) -> String {
    format!("{text}{}", " ".repeat(width.saturating_sub(text.width())))
}

fn pad_left(text: &str, width: usize) -> String {
    format!("{}{text}", " ".repeat(width.saturating_sub(text.width())))
}

fn center(text: &str, width: usize) -> String {
    let pad = width.saturating_sub(text.width());
    let left = pad / 2;
    format!("{}{text}{}", " ".repeat(left), " ".repeat(pad - left))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_bordered_table() {
        let sheet = Sheet::new(
            "S",
            vec![
                vec![CellValue::Text("ab".into()), CellValue::Number(120.0)],
                vec![CellValue::Text("c".into()), CellValue::Number(7.0)],
            ],
        );
        let expected = "\
Sheet: S (1/1)

┌───┬────┬─────┐
│   │ A  │  B  │
├───┼────┼─────┤
│ 1 │ ab │ 120 │
│ 2 │ c  │   7 │
└───┴────┴─────┘
";
        assert_eq!(render(&sheet, 0, 1), expected);
    }

    #[test]
    fn cjk_keeps_lines_aligned() {
        let sheet = Sheet::new(
            "日本語",
            vec![
                vec![CellValue::Text("あ".into())],
                vec![CellValue::Text("ああああ".into())],
            ],
        );
        let out = render(&sheet, 2, 5);
        assert!(out.contains("Sheet: 日本語 (3/5)"));
        let widths: Vec<usize> = out.lines().skip(2).map(|l| l.width()).collect();
        assert!(
            widths.iter().all(|w| *w == widths[0]),
            "uneven line widths: {widths:?}"
        );
    }

    #[test]
    fn clips_long_text_by_display_width() {
        let long = "これはとても長い備考でありグリッド表示では切り詰められるはずの文字列";
        let clipped = clip(long);
        assert!(clipped.ends_with('…'));
        assert!(clipped.width() <= MAX_CELL_WIDTH);
    }

    #[test]
    fn empty_sheet_has_placeholder() {
        let sheet = Sheet::new("empty", vec![]);
        assert!(render(&sheet, 0, 1).contains("(empty sheet)"));
    }

    #[test]
    fn control_chars_stay_on_one_line() {
        let sheet = Sheet::new(
            "s",
            vec![vec![
                CellValue::Text("a\nb\tc".into()),
                CellValue::Number(1.0),
            ]],
        );
        let out = render(&sheet, 0, 1);
        assert!(out.contains("a⏎b c"));
        let widths: Vec<usize> = out.lines().skip(2).map(|l| l.width()).collect();
        assert!(
            widths.iter().all(|w| *w == widths[0]),
            "uneven line widths: {widths:?}"
        );
    }

    #[test]
    fn column_labels() {
        assert_eq!(column_label(0), "A");
        assert_eq!(column_label(25), "Z");
        assert_eq!(column_label(26), "AA");
        assert_eq!(column_label(51), "AZ");
        assert_eq!(column_label(701), "ZZ");
        assert_eq!(column_label(702), "AAA");
    }
}
