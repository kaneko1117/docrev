use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::domain::cell::CellValue;
use crate::domain::sheet::Sheet;

use super::text::{cell_text, center, clip, column_label, pad_left, pad_right, sanitize};

pub const CELL_WIDTH: usize = 12;
/// Header line + tab bar + status bar.
pub const CHROME_ROWS: u16 = 3;

pub struct GridView<'a> {
    pub sheet: &'a Sheet,
    pub sheet_names: Vec<&'a str>,
    pub active: usize,
    pub cursor: (usize, usize),
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Scroll {
    pub top: usize,
    pub left: usize,
}

pub fn draw(frame: &mut Frame, view: &GridView, scroll: &mut Scroll) {
    let [grid_area, tabs_area, status_area] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    draw_grid(frame, grid_area, view, scroll);
    draw_tabs(frame, tabs_area, view);
    draw_status(frame, status_area, view);
}

fn draw_grid(frame: &mut Frame, area: Rect, view: &GridView, scroll: &mut Scroll) {
    let sheet = view.sheet;
    if sheet.row_count() == 0 || sheet.col_count() == 0 {
        frame.render_widget(Paragraph::new("(empty sheet)"), area);
        return;
    }

    let header_style = Style::default().add_modifier(Modifier::DIM);
    let row_label_width = sheet.row_count().to_string().len().max(2);
    let rows_visible = area.height.saturating_sub(1) as usize;
    let cols_visible = (area.width as usize).saturating_sub(row_label_width + 1) / (CELL_WIDTH + 1);

    let (cursor_row, cursor_col) = view.cursor;
    follow_cursor(&mut scroll.top, cursor_row, rows_visible);
    follow_cursor(&mut scroll.left, cursor_col, cols_visible);

    let last_row = (scroll.top + rows_visible).min(sheet.row_count());
    let last_col = (scroll.left + cols_visible).min(sheet.col_count());

    let mut lines = Vec::with_capacity(rows_visible + 1);
    let mut header = vec![Span::styled(" ".repeat(row_label_width), header_style)];
    for col in scroll.left..last_col {
        header.push(Span::raw(" "));
        header.push(Span::styled(
            center(&column_label(col as u32), CELL_WIDTH),
            header_style,
        ));
    }
    lines.push(Line::from(header));

    for row in scroll.top..last_row {
        let mut spans = vec![Span::styled(
            pad_left(&(row + 1).to_string(), row_label_width),
            header_style,
        )];
        for col in scroll.left..last_col {
            spans.push(Span::raw(" "));
            let cell = sheet.cell(row, col);
            let clipped = clip(&cell_text(cell), CELL_WIDTH);
            let aligned = if matches!(cell, CellValue::Number(_)) {
                pad_left(&clipped, CELL_WIDTH)
            } else {
                pad_right(&clipped, CELL_WIDTH)
            };
            let style = if (row, col) == view.cursor {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            spans.push(Span::styled(aligned, style));
        }
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

/// Keeps the cursor inside the visible window by moving the scroll origin.
fn follow_cursor(origin: &mut usize, cursor: usize, visible: usize) {
    if visible == 0 {
        return;
    }
    if cursor < *origin {
        *origin = cursor;
    } else if cursor >= *origin + visible {
        *origin = cursor + 1 - visible;
    }
}

fn draw_tabs(frame: &mut Frame, area: Rect, view: &GridView) {
    let mut spans = Vec::with_capacity(view.sheet_names.len());
    for (i, name) in view.sheet_names.iter().enumerate() {
        let style = if i == view.active {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default().add_modifier(Modifier::DIM)
        };
        spans.push(Span::styled(format!("[{name}]"), style));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_status(frame: &mut Frame, area: Rect, view: &GridView) {
    let (row, col) = view.cursor;
    let value = sanitize(&cell_text(view.sheet.cell(row, col)));
    let address = format!("{}{}", column_label(col as u32), row + 1);
    let left = format!("{address}: {value}");
    let hint = "q:quit  Tab:sheet";
    let gap = (area.width as usize).saturating_sub(
        unicode_width::UnicodeWidthStr::width(left.as_str())
            + unicode_width::UnicodeWidthStr::width(hint),
    );
    let line = Line::from(vec![
        Span::raw(left),
        Span::raw(" ".repeat(gap)),
        Span::styled(hint, Style::default().add_modifier(Modifier::DIM)),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;

    use super::*;

    fn sheet_3x3() -> Sheet {
        Sheet::new(
            "売上",
            vec![
                vec![
                    CellValue::Text("項目".into()),
                    CellValue::Text("単価".into()),
                    CellValue::Text("数量".into()),
                ],
                vec![
                    CellValue::Text("りんご".into()),
                    CellValue::Number(120.0),
                    CellValue::Number(3.0),
                ],
                vec![
                    CellValue::Text("みかん".into()),
                    CellValue::Number(80.0),
                    CellValue::Number(5.0),
                ],
            ],
        )
    }

    fn render_text(view: &GridView, scroll: &mut Scroll, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| draw(f, view, scroll)).unwrap();
        buffer_text(terminal.backend().buffer())
    }

    fn buffer_text(buffer: &Buffer) -> String {
        (0..buffer.area.height)
            .map(|y| {
                let mut line = String::new();
                let mut x = 0;
                // wide graphemes occupy a continuation cell — skip it
                while x < buffer.area.width {
                    let symbol = buffer.cell((x, y)).map(|c| c.symbol()).unwrap_or(" ");
                    line.push_str(symbol);
                    x += unicode_width::UnicodeWidthStr::width(symbol).max(1) as u16;
                }
                line.trim_end().to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn renders_grid_tabs_and_status() {
        let sheet = sheet_3x3();
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["売上", "経費"],
            active: 0,
            cursor: (1, 1),
        };
        let mut scroll = Scroll::default();
        insta::assert_snapshot!(render_text(&view, &mut scroll, 46, 7));
    }

    #[test]
    fn scrolls_to_keep_cursor_visible() {
        let rows: Vec<Vec<CellValue>> = (0..100)
            .map(|r| vec![CellValue::Number(f64::from(r))])
            .collect();
        let sheet = Sheet::new("big", rows);
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["big"],
            active: 0,
            cursor: (50, 0),
        };
        let mut scroll = Scroll::default();
        let text = render_text(&view, &mut scroll, 30, 6);
        assert!(text.contains("51"), "cursor row must be visible:\n{text}");
        assert_eq!(scroll.top, 48, "3 grid rows visible above chrome");
    }

    #[test]
    fn cursor_cell_is_reversed() {
        let sheet = sheet_3x3();
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["売上"],
            active: 0,
            cursor: (0, 0),
        };
        let mut terminal = Terminal::new(TestBackend::new(46, 7)).unwrap();
        terminal
            .draw(|f| draw(f, &view, &mut Scroll::default()))
            .unwrap();
        let cell = terminal.backend().buffer().cell((3, 1)).unwrap();
        assert!(cell.style().add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn empty_sheet_has_placeholder() {
        let sheet = Sheet::new("empty", vec![]);
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["empty"],
            active: 0,
            cursor: (0, 0),
        };
        let text = render_text(&view, &mut Scroll::default(), 30, 5);
        assert!(text.contains("(empty sheet)"));
    }
}
