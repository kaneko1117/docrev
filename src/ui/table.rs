use unicode_width::UnicodeWidthStr;

use crate::domain::anchor::Anchor;
use crate::domain::cell::CellValue;
use crate::domain::sheet::Sheet;

use super::text::{cell_lines, cell_text, center, clip, pad_left, pad_right, sanitize, wrap};

fn column_label(index: u32) -> String {
    Anchor::column_label(index)
}

const MAX_CELL_WIDTH: usize = 24;

/// `formulas` shows formula cells as `=…` instead of their results.
pub fn render(sheet: &Sheet, position: usize, total: usize, formulas: bool) -> String {
    let mut out = format!("Sheet: {} ({}/{})\n\n", sheet.name(), position + 1, total);
    let rows: Vec<usize> = sheet.visible_rows().collect();
    let cols: Vec<usize> = sheet.visible_cols().collect();
    if rows.is_empty() || cols.is_empty() {
        out.push_str("(empty sheet)\n");
        return out;
    }

    let texts: Vec<Vec<Vec<String>>> = rows
        .iter()
        .map(|&r| {
            cols.iter()
                .map(|&c| {
                    if formulas && let Some(formula) = sheet.formula_at(r, c) {
                        return wrap(&sanitize(&format!("={formula}")), MAX_CELL_WIDTH);
                    }
                    let cell = sheet.cell(r, c);
                    if matches!(
                        cell,
                        CellValue::Number(_) | CellValue::FormattedNumber { .. }
                    ) {
                        vec![clip(&cell_text(cell), MAX_CELL_WIDTH)]
                    } else {
                        cell_lines(cell, MAX_CELL_WIDTH)
                    }
                })
                .collect()
        })
        .collect();

    let row_label_width = sheet.row_count().to_string().len();
    let col_widths: Vec<usize> = cols
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let body = texts
                .iter()
                .filter_map(|row| row.get(i))
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
    for (&c, w) in cols.iter().zip(&col_widths) {
        out.push('│');
        out.push_str(&format!(" {} ", center(&column_label(c as u32), *w)));
    }
    out.push_str("│\n");

    out.push_str(&border('├', '┼', '┤'));

    for (&r, row) in rows.iter().zip(&texts) {
        let height = row.iter().map(Vec::len).max().unwrap_or(1);
        for sub in 0..height {
            out.push('│');
            if sub == 0 {
                out.push_str(&format!(" {:>rw$} ", r + 1, rw = row_label_width));
            } else {
                out.push_str(&format!(" {} ", " ".repeat(row_label_width)));
            }
            for ((&c, lines), &w) in cols.iter().zip(row).zip(&col_widths) {
                let text = lines.get(sub).map(String::as_str).unwrap_or("");
                // formulas align left whatever their result type
                let shows_formula = formulas && sheet.formula_at(r, c).is_some();
                let aligned = if !shows_formula
                    && matches!(
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
    fn hidden_rows_and_columns_are_left_out_with_true_labels() {
        use std::collections::HashSet;
        let sheet = Sheet::new(
            "S",
            vec![
                vec![
                    CellValue::Text("a1".into()),
                    CellValue::Text("b1".into()),
                    CellValue::Text("c1".into()),
                ],
                vec![
                    CellValue::Text("a2".into()),
                    CellValue::Text("b2".into()),
                    CellValue::Text("c2".into()),
                ],
                vec![
                    CellValue::Text("a3".into()),
                    CellValue::Text("b3".into()),
                    CellValue::Text("c3".into()),
                ],
            ],
        )
        .with_hidden_cols(HashSet::from([1]))
        .with_hidden_rows(HashSet::from([1]));
        let out = render(&sheet, 0, 1, false);
        assert!(out.contains("│ A  │ C  │"), "{out}");
        assert!(out.contains("│ 1 │ a1 │ c1 │"), "{out}");
        assert!(out.contains("│ 3 │ a3 │ c3 │"), "{out}");
        assert!(!out.contains("b1") && !out.contains("a2"), "{out}");
        let all_hidden = Sheet::new("S", vec![vec![CellValue::Number(1.0)]])
            .with_hidden_rows(HashSet::from([0]));
        assert!(render(&all_hidden, 0, 1, false).contains("(empty sheet)"));
    }

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
        assert_eq!(render(&sheet, 0, 1, false), expected);
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
        let out = render(&sheet, 2, 5, false);
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
        assert!(render(&sheet, 0, 1, false).contains("(empty sheet)"));
    }

    #[test]
    fn line_breaks_make_rows_tall_and_other_controls_stay_inline() {
        let sheet = Sheet::new(
            "s",
            vec![vec![
                CellValue::Text("a\nb\tc\r\nd".into()),
                CellValue::Number(1.0),
            ]],
        );
        let out = render(&sheet, 0, 1, false);
        let body: Vec<&str> = out.lines().skip(5).take(3).collect();
        assert!(body[0].contains("│ a   │ 1 │"), "{out}");
        assert!(body[1].contains("│ b c │   │"), "the tab is a space: {out}");
        assert!(body[2].contains("│ d   │   │"), "CRLF is one break: {out}");
        let widths: Vec<usize> = out.lines().skip(2).map(|l| l.width()).collect();
        assert!(
            widths.iter().all(|w| *w == widths[0]),
            "uneven line widths: {widths:?}"
        );
    }
}
