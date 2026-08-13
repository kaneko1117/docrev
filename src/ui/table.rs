use unicode_width::UnicodeWidthStr;

use crate::domain::anchor::Anchor;
use crate::domain::cell::CellValue;
use crate::domain::sheet::Sheet;

use super::text::{cell_text, center, clip, pad_left, pad_right, wrap};

fn column_label(index: u32) -> String {
    Anchor::column_label(index)
}

const MAX_CELL_WIDTH: usize = 24;

pub fn render(sheet: &Sheet, position: usize, total: usize) -> String {
    let mut out = format!("Sheet: {} ({}/{})\n\n", sheet.name(), position + 1, total);
    let col_count = sheet.col_count();
    if sheet.row_count() == 0 || col_count == 0 {
        out.push_str("(empty sheet)\n");
        return out;
    }

    // wrapped lines per cell: text wraps at the cell-width cap (#33),
    // numbers stay on one clipped line so digit groups never split
    let texts: Vec<Vec<Vec<String>>> = (0..sheet.row_count())
        .map(|r| {
            (0..col_count)
                .map(|c| {
                    let cell = sheet.cell(r, c);
                    let text = cell_text(cell);
                    if matches!(
                        cell,
                        CellValue::Number(_) | CellValue::FormattedNumber { .. }
                    ) {
                        vec![clip(&text, MAX_CELL_WIDTH)]
                    } else {
                        wrap(&text, MAX_CELL_WIDTH)
                    }
                })
                .collect()
        })
        .collect();

    let row_label_width = sheet.row_count().to_string().len();
    let col_widths: Vec<usize> = (0..col_count)
        .map(|c| {
            let body = texts
                .iter()
                .filter_map(|row| row.get(c))
                .flat_map(|lines| lines.iter())
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
        let height = row.iter().map(Vec::len).max().unwrap_or(1);
        for sub in 0..height {
            out.push('│');
            if sub == 0 {
                out.push_str(&format!(" {:>rw$} ", r + 1, rw = row_label_width));
            } else {
                out.push_str(&format!(" {} ", " ".repeat(row_label_width)));
            }
            for (c, lines) in row.iter().enumerate() {
                let w = col_widths.get(c).copied().unwrap_or(0);
                let text = lines.get(sub).map(String::as_str).unwrap_or("");
                let aligned = if matches!(
                    sheet.cell(r, c),
                    CellValue::Number(_) | CellValue::FormattedNumber { .. }
                ) {
                    pad_left(text, w)
                } else {
                    pad_right(text, w)
                };
                out.push('│');
                out.push_str(&format!(" {aligned} "));
            }
            out.push_str("│\n");
        }
    }

    out.push_str(&border('└', '┴', '┘'));
    out
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
}
