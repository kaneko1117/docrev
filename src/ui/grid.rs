use std::collections::HashSet;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::domain::anchor::Anchor;
use crate::domain::cell::CellValue;
use crate::domain::comment::CommentThread;
use crate::domain::number_format::FormatColor;
use crate::domain::sheet::{Rgb, Sheet};

use super::text::{cell_text, center, clip, pad_left, pad_right, sanitize};

pub const DEFAULT_CELL_WIDTH: usize = 12;
/// Formula bar + header line + tab bar + status bar.
pub const CHROME_ROWS: u16 = 4;
const PANEL_WIDTH: u16 = 32;

// Sheets-flavored palette, painted regardless of terminal theme (#16).
const TEXT: Color = Color::Rgb(32, 33, 36);
const CANVAS_BG: Color = Color::Rgb(255, 255, 255);
const HEADER_BG: Color = Color::Rgb(241, 243, 244);
const HEADER_FG: Color = Color::Rgb(95, 99, 104);
const SELECTION_BG: Color = Color::Rgb(210, 227, 252);
const MARKER_FG: Color = Color::Rgb(242, 153, 0);
const NOTICE_FG: Color = Color::Rgb(217, 48, 37);
const GRIDLINE: Color = Color::Rgb(218, 220, 224);
const USER_FG: Color = Color::Rgb(146, 64, 14);
const AGENT_FG: Color = Color::Rgb(11, 87, 208);

fn canvas() -> Style {
    Style::new().bg(CANVAS_BG).fg(TEXT)
}

fn header() -> Style {
    Style::new().bg(HEADER_BG).fg(HEADER_FG)
}

fn selected() -> Style {
    Style::new().bg(SELECTION_BG).fg(TEXT)
}

/// Horizontal gridlines without spending screen rows: a colored underline.
fn ruled(style: Style) -> Style {
    style
        .add_modifier(Modifier::UNDERLINED)
        .underline_color(GRIDLINE)
}

/// Number-format colors tuned for the white canvas: yellow and white as-is
/// would be unreadable, so they get darkened stand-ins.
fn format_fg(color: FormatColor) -> Color {
    match color {
        FormatColor::Red => Color::Rgb(217, 48, 37),
        FormatColor::Blue => Color::Rgb(11, 87, 208),
        FormatColor::Green => Color::Rgb(19, 115, 51),
        FormatColor::Yellow => Color::Rgb(178, 138, 0),
        FormatColor::Magenta => Color::Rgb(168, 37, 168),
        FormatColor::Cyan => Color::Rgb(0, 131, 143),
        FormatColor::Black => TEXT,
        FormatColor::White => Color::Rgb(128, 134, 139),
    }
}

/// Text color precedence, matching Excel: a color from the number format
/// (`[Red]` sections) wins over the cell's font color.
fn cell_fg(cell: &CellValue, font: Option<Rgb>, style: Style) -> Style {
    match (cell, font) {
        (
            CellValue::FormattedNumber {
                color: Some(color), ..
            },
            _,
        ) => style.fg(format_fg(*color)),
        (_, Some(c)) => style.fg(Color::Rgb(c.r, c.g, c.b)),
        _ => style,
    }
}

/// Canvas painted with the workbook fill when the cell has one.
fn filled_canvas(fill: Option<Rgb>) -> Style {
    match fill {
        Some(f) => canvas().bg(Color::Rgb(f.r, f.g, f.b)),
        None => canvas(),
    }
}

fn column_label(index: u32) -> String {
    Anchor::column_label(index)
}

pub struct GridView<'a> {
    pub sheet: &'a Sheet,
    pub sheet_names: Vec<&'a str>,
    pub active: usize,
    pub cursor: (usize, usize),
    /// Cells with an unresolved comment thread, marked with `●`.
    pub markers: HashSet<(usize, usize)>,
    pub notice: Option<&'a str>,
    /// Thread under the cursor — shown in the side panel.
    pub thread: Option<&'a CommentThread>,
    /// Comment editor state — shown as a popup.
    pub editor: Option<EditorView<'a>>,
    /// Per-column display widths; missing entries use `DEFAULT_CELL_WIDTH`.
    pub col_widths: Vec<usize>,
}

pub struct EditorView<'a> {
    pub title: String,
    pub buffer: &'a str,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Scroll {
    pub top: usize,
    pub left: usize,
}

pub fn draw(frame: &mut Frame, view: &GridView, scroll: &mut Scroll) {
    let [formula_area, main_area, tabs_area, status_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let (grid_area, panel_area) = match view.thread {
        Some(_) if main_area.width > PANEL_WIDTH + 20 => {
            let [g, p] = Layout::horizontal([Constraint::Min(20), Constraint::Length(PANEL_WIDTH)])
                .areas(main_area);
            (g, Some(p))
        }
        _ => (main_area, None),
    };
    draw_formula_bar(frame, formula_area, view);
    draw_grid(frame, grid_area, view, scroll);
    if let (Some(panel), Some(thread)) = (panel_area, view.thread) {
        draw_panel(frame, panel, thread);
    }
    draw_tabs(frame, tabs_area, view);
    draw_status(frame, status_area, view);
    if let Some(editor) = &view.editor {
        draw_editor(frame, editor);
    }
}

/// Sheets' name box + formula bar: `B2      │ 120`; a merged region shows
/// its range and the anchor value.
fn draw_formula_bar(frame: &mut Frame, area: Rect, view: &GridView) {
    let (row, col) = view.cursor;
    let (address, value) = match view.sheet.merge_at(row, col) {
        Some(merge) => {
            let start = Anchor::cell("", merge.start_row as u32, merge.start_col as u32);
            let end = Anchor::cell("", merge.end_row as u32, merge.end_col as u32);
            let (anchor_row, anchor_col) = merge.anchor();
            (
                format!("{}:{}", start.cell_ref(), end.cell_ref()),
                sanitize(&cell_text(view.sheet.cell(anchor_row, anchor_col))),
            )
        }
        None => (
            format!("{}{}", column_label(col as u32), row + 1),
            sanitize(&cell_text(view.sheet.cell(row, col))),
        ),
    };
    let line = Line::from(vec![
        Span::styled(
            format!(" {address:<7}"),
            canvas().add_modifier(Modifier::BOLD),
        ),
        Span::styled("│ ", Style::new().bg(CANVAS_BG).fg(HEADER_FG)),
        Span::styled(value, canvas()),
    ]);
    frame.render_widget(Paragraph::new(line).style(canvas()), area);
}

fn draw_grid(frame: &mut Frame, area: Rect, view: &GridView, scroll: &mut Scroll) {
    let sheet = view.sheet;
    if sheet.row_count() == 0 || sheet.col_count() == 0 {
        frame.render_widget(Paragraph::new("(empty sheet)").style(canvas()), area);
        return;
    }

    let header_style = header();
    let row_label_width = sheet.row_count().to_string().len().max(2);
    let rows_visible = area.height.saturating_sub(1) as usize;
    let avail = (area.width as usize).saturating_sub(row_label_width);
    let width_of = |c: usize| {
        view.col_widths
            .get(c)
            .copied()
            .unwrap_or(DEFAULT_CELL_WIDTH)
    };

    let (cursor_row, cursor_col) = view.cursor;
    follow_cursor(&mut scroll.top, cursor_row, rows_visible);
    follow_col(&mut scroll.left, cursor_col, avail, &width_of);

    let last_row = (scroll.top + rows_visible).min(sheet.row_count());
    let last_col = last_visible_col(scroll.left, sheet.col_count(), avail, &width_of);

    let mut lines = Vec::with_capacity(rows_visible + 1);
    let mut header_line = vec![Span::styled(
        " ".repeat(row_label_width),
        ruled(header_style),
    )];
    for col in scroll.left..last_col {
        header_line.push(Span::styled("│", ruled(header().fg(GRIDLINE))));
        header_line.push(Span::styled(
            center(&column_label(col as u32), width_of(col)),
            ruled(header_style),
        ));
    }
    lines.push(Line::from(header_line));

    for row in scroll.top..last_row {
        let mut spans = vec![Span::styled(
            pad_left(&(row + 1).to_string(), row_label_width),
            ruled(header_style),
        )];
        let mut col = scroll.left;
        while col < last_col {
            // A merged region renders as one cell: the value on its anchor
            // row spanning all its columns, no gridlines inside, and the
            // whole region highlights when the cursor is anywhere in it.
            // A thread on ANY of its cells shows one ● on the first
            // visible row.
            if let Some(merge) = sheet.merge_at(row, col) {
                let first_visible_row = merge.start_row.max(scroll.top);
                let region_marked = row == first_visible_row
                    && view.markers.iter().any(|(r, c)| merge.contains(*r, *c));
                if region_marked {
                    spans.push(Span::styled(
                        "●",
                        ruled(Style::new().bg(CANVAS_BG).fg(MARKER_FG)),
                    ));
                } else {
                    spans.push(Span::styled("│", ruled(canvas().fg(GRIDLINE))));
                }
                let segment_end = (merge.end_col + 1).min(last_col);
                let span_cols = segment_end - col;
                let span_width: usize =
                    (col..segment_end).map(&width_of).sum::<usize>() + (span_cols - 1);
                let (anchor_row, anchor_col) = merge.anchor();
                let anchor_cell = sheet.cell(anchor_row, anchor_col);
                let text = if row == anchor_row {
                    cell_text(anchor_cell)
                } else {
                    String::new()
                };
                let aligned = pad_right(&clip(&text, span_width), span_width);
                let base = if merge.contains(cursor_row, cursor_col) {
                    selected()
                } else {
                    // the anchor's fill paints the whole merged region
                    filled_canvas(sheet.fill_at(anchor_row, anchor_col))
                };
                let base = if row == anchor_row {
                    // font color stays off while the cursor is inside so
                    // light fonts remain readable on the selection blue
                    let font = if merge.contains(cursor_row, cursor_col) {
                        None
                    } else {
                        sheet.font_color_at(anchor_row, anchor_col)
                    };
                    cell_fg(anchor_cell, font, base)
                } else {
                    base
                };
                // the horizontal gridline only under the region's last row
                let style = if row == merge.end_row {
                    ruled(base)
                } else {
                    base
                };
                spans.push(Span::styled(aligned, style));
                col += span_cols;
                continue;
            }

            let fill = sheet.fill_at(row, col);
            if view.markers.contains(&(row, col)) {
                spans.push(Span::styled("●", ruled(filled_canvas(fill).fg(MARKER_FG))));
            } else {
                spans.push(Span::styled("│", ruled(canvas().fg(GRIDLINE))));
            }
            let cell = sheet.cell(row, col);
            let text = cell_text(cell);
            let own_width = width_of(col);
            let is_number = matches!(
                cell,
                CellValue::Number(_) | CellValue::FormattedNumber { .. }
            );
            let on_cursor = (row, col) == view.cursor;

            // Sheets-style overflow: text wider than its column spills over
            // empty neighbors (never numbers; a marker, data, a merged
            // region, a fill or the cursor stops it; disabled on the cursor
            // and on filled cells so their boxes keep clean edges).
            let mut span_cols = 1;
            let mut span_width = own_width;
            if !is_number
                && !on_cursor
                && fill.is_none()
                && unicode_width::UnicodeWidthStr::width(text.as_str()) > own_width
            {
                let mut next = col + 1;
                while next < last_col
                    && unicode_width::UnicodeWidthStr::width(text.as_str()) > span_width
                    && sheet.cell(row, next).is_empty()
                    && sheet.merge_at(row, next).is_none()
                    && sheet.fill_at(row, next).is_none()
                    && !view.markers.contains(&(row, next))
                    && (row, next) != view.cursor
                {
                    span_width += 1 + width_of(next);
                    span_cols += 1;
                    next += 1;
                }
            }

            let clipped = clip(&text, span_width);
            let aligned = if is_number {
                pad_left(&clipped, span_width)
            } else {
                pad_right(&clipped, span_width)
            };
            // font color stays off on the cursor cell so light fonts remain
            // readable on the selection blue
            let font = if on_cursor {
                None
            } else {
                sheet.font_color_at(row, col)
            };
            let style = cell_fg(
                cell,
                font,
                if on_cursor {
                    selected()
                } else {
                    filled_canvas(fill)
                },
            );
            spans.push(Span::styled(aligned, ruled(style)));
            col += span_cols;
        }
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines).style(canvas()), area);
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

/// Horizontal variant for variable column widths.
fn follow_col(left: &mut usize, cursor: usize, avail: usize, width_of: &impl Fn(usize) -> usize) {
    if cursor < *left {
        *left = cursor;
        return;
    }
    while *left < cursor {
        let span: usize = (*left..=cursor).map(|c| width_of(c) + 1).sum();
        if span <= avail {
            break;
        }
        *left += 1;
    }
}

/// First column that no longer fits; always shows at least one column.
fn last_visible_col(
    left: usize,
    col_count: usize,
    avail: usize,
    width_of: &impl Fn(usize) -> usize,
) -> usize {
    let mut used = 0;
    let mut col = left;
    while col < col_count {
        let needed = width_of(col) + 1;
        if used + needed > avail && col > left {
            break;
        }
        used += needed;
        col += 1;
    }
    col
}

fn draw_panel(frame: &mut Frame, area: Rect, thread: &CommentThread) {
    let title = if thread.resolved {
        format!("{} (resolved)", thread.anchor.cell_ref())
    } else {
        thread.anchor.cell_ref()
    };
    let mut lines = vec![
        Line::styled(title, canvas().add_modifier(Modifier::BOLD)),
        Line::raw(""),
    ];
    push_message(&mut lines, &thread.author, &thread.body);
    for reply in &thread.replies {
        lines.push(Line::raw(""));
        push_message(&mut lines, &reply.author, &reply.body);
    }
    let panel = Paragraph::new(lines)
        .style(canvas())
        .wrap(Wrap { trim: false })
        .block(
            Block::new()
                .borders(Borders::LEFT)
                .border_style(Style::new().bg(CANVAS_BG).fg(HEADER_FG)),
        );
    frame.render_widget(panel, area);
}

fn push_message(lines: &mut Vec<Line>, author: &str, body: &str) {
    let color = if author == "user" { USER_FG } else { AGENT_FG };
    lines.push(Line::styled(
        format!(" {author}:"),
        Style::new().bg(CANVAS_BG).fg(color),
    ));
    for part in body.split('\n') {
        lines.push(Line::raw(format!("  {}", sanitize(part))));
    }
}

fn draw_editor(frame: &mut Frame, editor: &EditorView) {
    let area = frame.area();
    let width = area.width.saturating_sub(4).clamp(20, 50);
    let inner_width = width.saturating_sub(2).max(1) as usize;
    let text_lines: Vec<String> = editor.buffer.split('\n').map(sanitize).collect();

    // height must count *wrapped* rows, or long lines push the cursor and
    // the hint out of the box; if the screen is smaller still, scroll so
    // the cursor end stays visible
    let hint = "Enter:newline  Ctrl+S:save  Esc:cancel";
    let wrapped_rows = |columns: usize| columns.max(1).div_ceil(inner_width);
    let mut total_rows = wrapped_rows(unicode_width::UnicodeWidthStr::width(hint));
    for (i, line) in text_lines.iter().enumerate() {
        let mut columns = unicode_width::UnicodeWidthStr::width(line.as_str());
        if i == text_lines.len() - 1 {
            columns += 1; // the █ cursor
        }
        total_rows += wrapped_rows(columns);
    }

    let height = (total_rows as u16 + 2).clamp(5, area.height.saturating_sub(2).max(5));
    let inner_height = height.saturating_sub(2) as usize;
    let scroll = total_rows.saturating_sub(inner_height) as u16;
    let popup = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    let mut lines: Vec<Line> = Vec::with_capacity(text_lines.len() + 1);
    for (i, text) in text_lines.iter().enumerate() {
        if i == text_lines.len() - 1 {
            lines.push(Line::raw(format!("{text}█")));
        } else {
            lines.push(Line::raw(text.clone()));
        }
    }
    lines.push(Line::styled(hint, Style::new().bg(CANVAS_BG).fg(HEADER_FG)));
    let popup_widget = Paragraph::new(lines)
        .style(canvas())
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0))
        .block(
            Block::bordered()
                .title(editor.title.clone())
                .border_style(Style::new().bg(CANVAS_BG).fg(HEADER_FG)),
        );
    frame.render_widget(popup_widget, popup);
}

fn draw_tabs(frame: &mut Frame, area: Rect, view: &GridView) {
    let mut spans = Vec::with_capacity(view.sheet_names.len());
    for (i, name) in view.sheet_names.iter().enumerate() {
        let style = if i == view.active {
            canvas().add_modifier(Modifier::BOLD)
        } else {
            header()
        };
        spans.push(Span::styled(format!("[{name}]"), style));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)).style(header()), area);
}

/// Hints and notices only — the cell value lives in the formula bar now.
fn draw_status(frame: &mut Frame, area: Rect, view: &GridView) {
    let left = match view.notice {
        Some(notice) => format!("⚠ {notice}"),
        None => String::new(),
    };
    let hint = if view.thread.is_some() {
        "r:reply  c:comment  q:quit"
    } else {
        "c:comment  q:quit  Tab:sheet"
    };
    let gap = (area.width as usize).saturating_sub(
        unicode_width::UnicodeWidthStr::width(left.as_str())
            + unicode_width::UnicodeWidthStr::width(hint),
    );
    let line = Line::from(vec![
        Span::styled(left, Style::new().bg(HEADER_BG).fg(NOTICE_FG)),
        Span::styled(" ".repeat(gap), header()),
        Span::styled(hint, header()),
    ]);
    frame.render_widget(Paragraph::new(line).style(header()), area);
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
            markers: HashSet::new(),
            notice: None,
            thread: None,
            editor: None,
            col_widths: vec![],
        };
        let mut scroll = Scroll::default();
        insta::assert_snapshot!(render_text(&view, &mut scroll, 46, 7));
    }

    #[test]
    fn formatted_numbers_render_right_aligned_and_colored() {
        let sheet = Sheet::new(
            "書式",
            vec![vec![
                CellValue::FormattedNumber {
                    value: -1234.0,
                    text: "▲1,234".into(),
                    color: Some(FormatColor::Red),
                },
                CellValue::Number(5.0),
            ]],
        );
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["書式"],
            active: 0,
            cursor: (0, 1),
            markers: HashSet::new(),
            notice: None,
            thread: None,
            editor: None,
            col_widths: vec![],
        };
        let mut terminal = Terminal::new(TestBackend::new(46, 7)).unwrap();
        let mut scroll = Scroll::default();
        terminal.draw(|f| draw(f, &view, &mut scroll)).unwrap();
        let buffer = terminal.backend().buffer();

        let text = buffer_text(buffer);
        assert!(
            text.contains("▲1,234│"),
            "formatted number must sit flush right against the gridline:\n{text}"
        );

        let mut fg = None;
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                if let Some(cell) = buffer.cell((x, y))
                    && cell.symbol() == "▲"
                {
                    fg = Some(cell.fg);
                }
            }
        }
        assert_eq!(
            fg,
            Some(format_fg(FormatColor::Red)),
            "the [Red] tag must color the cell"
        );
    }

    #[test]
    fn font_colors_reach_the_terminal_but_format_colors_win() {
        use std::collections::HashMap;

        let white = Rgb {
            r: 255,
            g: 255,
            b: 255,
        };
        let navy = Rgb {
            r: 0x20,
            g: 0x38,
            b: 0x64,
        };
        let blue = Rgb { r: 0, g: 0, b: 255 };
        let sheet = Sheet::new(
            "フォント",
            vec![vec![
                CellValue::Text("白".into()),
                CellValue::FormattedNumber {
                    value: -5.0,
                    text: "▲5".into(),
                    color: Some(FormatColor::Red),
                },
                CellValue::Empty,
            ]],
        )
        .with_fills(HashMap::from([((0, 0), navy)]))
        .with_font_colors(HashMap::from([((0, 0), white), ((0, 1), blue)]));
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["フォント"],
            active: 0,
            cursor: (0, 2),
            markers: HashSet::new(),
            notice: None,
            thread: None,
            editor: None,
            col_widths: vec![],
        };
        let mut terminal = Terminal::new(TestBackend::new(46, 7)).unwrap();
        let mut scroll = Scroll::default();
        terminal.draw(|f| draw(f, &view, &mut scroll)).unwrap();
        let buffer = terminal.backend().buffer();

        let fg_of = |symbol: &str| {
            (0..buffer.area.width).find_map(|x| {
                (0..buffer.area.height).find_map(|y| {
                    buffer
                        .cell((x, y))
                        .filter(|c| c.symbol() == symbol)
                        .map(|c| c.fg)
                })
            })
        };
        assert_eq!(
            fg_of("白"),
            Some(Color::Rgb(255, 255, 255)),
            "the font color must reach the glyph"
        );
        assert_eq!(
            fg_of("▲"),
            Some(format_fg(FormatColor::Red)),
            "the [Red] format color beats the blue font"
        );
    }

    #[test]
    fn fills_paint_backgrounds_and_stop_the_spill() {
        use std::collections::HashMap;

        let long = "この文章はとても長いのでスピルするはずの文字列";
        let green = Rgb {
            r: 0x00,
            g: 0xB0,
            b: 0x50,
        };
        let sheet = Sheet::new(
            "塗り",
            vec![vec![
                CellValue::Text(long.into()),
                CellValue::Empty,
                CellValue::Empty,
            ]],
        )
        .with_fills(HashMap::from([((0, 1), green)]));
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["塗り"],
            active: 0,
            cursor: (0, 2),
            markers: HashSet::new(),
            notice: None,
            thread: None,
            editor: None,
            col_widths: vec![],
        };
        let mut terminal = Terminal::new(TestBackend::new(46, 7)).unwrap();
        let mut scroll = Scroll::default();
        terminal.draw(|f| draw(f, &view, &mut scroll)).unwrap();
        let buffer = terminal.backend().buffer();

        let painted = (0..buffer.area.width).any(|x| {
            (0..buffer.area.height).any(|y| {
                buffer
                    .cell((x, y))
                    .is_some_and(|c| c.bg == Color::Rgb(0x00, 0xB0, 0x50))
            })
        });
        assert!(painted, "the fill must reach the terminal background");

        // the filled neighbor stops the overflow, so the long text clips
        let text = buffer_text(buffer);
        assert!(
            text.contains('…'),
            "text must clip instead of spilling over the filled cell:\n{text}"
        );
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
            markers: HashSet::new(),
            notice: None,
            thread: None,
            editor: None,
            col_widths: vec![],
        };
        let mut scroll = Scroll::default();
        let text = render_text(&view, &mut scroll, 30, 6);
        assert!(text.contains("51"), "cursor row must be visible:\n{text}");
        assert_eq!(scroll.top, 49, "2 grid rows visible above chrome");
    }

    #[test]
    fn cursor_cell_uses_the_selection_color_on_the_white_canvas() {
        let sheet = sheet_3x3();
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["売上"],
            active: 0,
            cursor: (0, 0),
            markers: HashSet::new(),
            notice: None,
            thread: None,
            editor: None,
            col_widths: vec![],
        };
        let mut terminal = Terminal::new(TestBackend::new(46, 8)).unwrap();
        terminal
            .draw(|f| draw(f, &view, &mut Scroll::default()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        // row 0: formula bar, row 1: column headers, row 2: first data row
        let cursor_cell = buffer.cell((3, 2)).unwrap();
        assert_eq!(cursor_cell.style().bg, Some(SELECTION_BG));
        let plain_cell = buffer.cell((3, 3)).unwrap();
        assert_eq!(plain_cell.style().bg, Some(CANVAS_BG));
        let header_cell = buffer.cell((3, 1)).unwrap();
        assert_eq!(header_cell.style().bg, Some(HEADER_BG));
    }

    #[test]
    fn empty_sheet_has_placeholder() {
        let sheet = Sheet::new("empty", vec![]);
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["empty"],
            active: 0,
            cursor: (0, 0),
            markers: HashSet::new(),
            notice: None,
            thread: None,
            editor: None,
            col_widths: vec![],
        };
        let text = render_text(&view, &mut Scroll::default(), 30, 5);
        assert!(text.contains("(empty sheet)"));
    }

    fn sample_thread() -> CommentThread {
        use crate::domain::comment::Reply;
        CommentThread {
            id: "t1".into(),
            anchor: Anchor::cell("売上", 1, 1),
            author: "user".into(),
            body: "単価が古いのでは?".into(),
            created_at: "2026-08-11T09:15:00Z".into(),
            resolved: false,
            replies: vec![Reply {
                id: "r1".into(),
                author: "claude".into(),
                body: "確認しました。150円です".into(),
                created_at: "2026-08-11T09:20:00Z".into(),
            }],
        }
    }

    #[test]
    fn side_panel_shows_the_thread() {
        let sheet = sheet_3x3();
        let thread = sample_thread();
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["売上"],
            active: 0,
            cursor: (1, 1),
            markers: HashSet::from([(1, 1)]),
            notice: None,
            thread: Some(&thread),
            editor: None,
            col_widths: vec![],
        };
        let mut scroll = Scroll::default();
        insta::assert_snapshot!(render_text(&view, &mut scroll, 76, 10));
    }

    #[test]
    fn editor_popup_overlays_the_grid() {
        let sheet = sheet_3x3();
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["売上"],
            active: 0,
            cursor: (1, 1),
            markers: HashSet::new(),
            notice: None,
            thread: None,
            editor: Some(EditorView {
                title: " Comment on B2 ".into(),
                buffer: "line one\nline two",
            }),
            col_widths: vec![],
        };
        let mut scroll = Scroll::default();
        insta::assert_snapshot!(render_text(&view, &mut scroll, 50, 10));
    }

    fn overflow_sheet() -> Sheet {
        let long = "あいうえおかきくけこさしすせそ"; // display width 30
        Sheet::new(
            "OF",
            vec![
                vec![CellValue::Text(long.into())],
                vec![CellValue::Text(long.into()), CellValue::Text("X".into())],
                vec![CellValue::Number(1234567890123456.0), CellValue::Empty],
            ],
        )
    }

    #[test]
    fn overflow_spills_over_empty_cells_only() {
        let sheet = overflow_sheet();
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["OF"],
            active: 0,
            cursor: (2, 2),
            markers: HashSet::new(),
            notice: None,
            thread: None,
            editor: None,
            col_widths: vec![],
        };
        let mut scroll = Scroll::default();
        insta::assert_snapshot!(render_text(&view, &mut scroll, 50, 8));
    }

    #[test]
    fn cursor_on_the_source_cell_suppresses_overflow() {
        let sheet = overflow_sheet();
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["OF"],
            active: 0,
            cursor: (0, 0),
            markers: HashSet::new(),
            notice: None,
            thread: None,
            editor: None,
            col_widths: vec![],
        };
        let text = render_text(&view, &mut Scroll::default(), 50, 8);
        let grid_row = text
            .lines()
            .find(|l| l.starts_with(" 1│"))
            .expect("first data row");
        assert!(
            !grid_row.contains("さしすせそ"),
            "overflow must be clipped while the cursor sits on the cell:\n{text}"
        );
    }

    #[test]
    fn a_marker_blocks_overflow() {
        let sheet = overflow_sheet();
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["OF"],
            active: 0,
            cursor: (2, 2),
            markers: HashSet::from([(0, 1)]),
            notice: None,
            thread: None,
            editor: None,
            col_widths: vec![],
        };
        let text = render_text(&view, &mut Scroll::default(), 50, 8);
        assert!(text.contains('●'));
        assert!(
            !text.contains("さしすせそ"),
            "overflow must stop at a commented cell:\n{text}"
        );
    }

    #[test]
    fn custom_column_widths_change_the_grid() {
        let sheet = Sheet::new(
            "W",
            vec![vec![
                CellValue::Text("幅20の列です".into()),
                CellValue::Number(42.0),
                CellValue::Text("ok".into()),
            ]],
        );
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["W"],
            active: 0,
            cursor: (0, 2),
            markers: HashSet::new(),
            notice: None,
            thread: None,
            editor: None,
            col_widths: vec![20, 6, 8],
        };
        let mut scroll = Scroll::default();
        insta::assert_snapshot!(render_text(&view, &mut scroll, 50, 6));
    }

    #[test]
    fn editor_keeps_cursor_and_hint_visible_with_long_wrapped_text() {
        let long = "x".repeat(120);
        let sheet = sheet_3x3();
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["売上"],
            active: 0,
            cursor: (0, 0),
            markers: HashSet::new(),
            notice: None,
            thread: None,
            editor: Some(EditorView {
                title: " Comment on A1 ".into(),
                buffer: &long,
            }),
            col_widths: vec![],
        };
        let text = render_text(&view, &mut Scroll::default(), 50, 10);
        assert!(text.contains('█'), "cursor must stay visible:\n{text}");
        assert!(
            text.contains("Esc:cancel"),
            "hint must stay visible:\n{text}"
        );
    }

    fn merged_sheet() -> Sheet {
        use crate::domain::sheet::MergedRange;
        Sheet::new(
            "M",
            vec![
                vec![CellValue::Text("2026年度 売上報告".into())],
                vec![
                    CellValue::Text("上期".into()),
                    CellValue::Number(100.0),
                    CellValue::Text("備考".into()),
                ],
                vec![CellValue::Empty, CellValue::Number(200.0)],
            ],
        )
        .with_merges(vec![
            MergedRange {
                start_row: 0,
                start_col: 0,
                end_row: 0,
                end_col: 2,
            },
            MergedRange {
                start_row: 1,
                start_col: 0,
                end_row: 2,
                end_col: 0,
            },
        ])
    }

    #[test]
    fn merged_regions_render_as_single_cells() {
        let sheet = merged_sheet();
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["M"],
            active: 0,
            cursor: (1, 1),
            markers: HashSet::new(),
            notice: None,
            thread: None,
            editor: None,
            col_widths: vec![],
        };
        let mut scroll = Scroll::default();
        insta::assert_snapshot!(render_text(&view, &mut scroll, 50, 8));
    }

    #[test]
    fn cursor_inside_a_merge_highlights_the_whole_region() {
        let sheet = merged_sheet();
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["M"],
            active: 0,
            cursor: (0, 2), // C1, inside A1:C1
            markers: HashSet::new(),
            notice: None,
            thread: None,
            editor: None,
            col_widths: vec![],
        };
        let mut terminal = Terminal::new(TestBackend::new(50, 8)).unwrap();
        terminal
            .draw(|f| draw(f, &view, &mut Scroll::default()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        // row 2 is the merged A1:C1 row; column x=3 is inside column A
        let cell = buffer.cell((3, 2)).unwrap();
        assert_eq!(cell.style().bg, Some(SELECTION_BG));
        // formula bar shows the range
        let top: String = (0..20)
            .map(|x| buffer.cell((x, 0)).map(|c| c.symbol()).unwrap_or(" "))
            .collect::<Vec<_>>()
            .join("");
        assert!(top.contains("A1:C1"), "formula bar: {top}");
    }

    #[test]
    fn vertical_merge_draws_the_gridline_only_under_its_last_row() {
        let sheet = merged_sheet();
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["M"],
            active: 0,
            cursor: (1, 2),
            markers: HashSet::new(),
            notice: None,
            thread: None,
            editor: None,
            col_widths: vec![],
        };
        let mut terminal = Terminal::new(TestBackend::new(50, 8)).unwrap();
        terminal
            .draw(|f| draw(f, &view, &mut Scroll::default()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        // A2:A3 merge: row 3 (A2, mid-merge) has no underline, row 4 (A3, last) does
        let mid = buffer.cell((3, 3)).unwrap();
        assert!(!mid.style().add_modifier.contains(Modifier::UNDERLINED));
        let last = buffer.cell((3, 4)).unwrap();
        assert!(last.style().add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn a_thread_on_an_interior_merge_cell_still_shows_a_marker() {
        let sheet = merged_sheet();
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["M"],
            active: 0,
            cursor: (2, 2),
            markers: HashSet::from([(0, 1)]), // B1, interior of A1:C1
            notice: None,
            thread: None,
            editor: None,
            col_widths: vec![],
        };
        let text = render_text(&view, &mut Scroll::default(), 50, 8);
        let merged_row = text
            .lines()
            .find(|l| l.starts_with(" 1"))
            .expect("merged row");
        assert!(
            merged_row.contains('●'),
            "the region must carry the marker:\n{text}"
        );
    }

    #[test]
    fn overflow_does_not_spill_into_a_merge() {
        use crate::domain::sheet::MergedRange;
        let sheet = Sheet::new(
            "M",
            vec![vec![
                CellValue::Text("あいうえおかきくけこさしすせそ".into()),
                CellValue::Empty,
            ]],
        )
        .with_merges(vec![MergedRange {
            start_row: 0,
            start_col: 1,
            end_row: 0,
            end_col: 1,
        }]);
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["M"],
            active: 0,
            cursor: (0, 1),
            markers: HashSet::new(),
            notice: None,
            thread: None,
            editor: None,
            col_widths: vec![],
        };
        let text = render_text(&view, &mut Scroll::default(), 50, 6);
        let grid_row = text
            .lines()
            .find(|l| l.starts_with(" 1│"))
            .expect("data row");
        assert!(
            !grid_row.contains("さしすせそ"),
            "must not overflow into the merged cell:\n{text}"
        );
    }

    #[test]
    fn markers_and_notice_are_rendered() {
        let sheet = sheet_3x3();
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["売上"],
            active: 0,
            cursor: (0, 0),
            markers: HashSet::from([(1, 1)]),
            notice: Some("comments unavailable: invalid sidecar"),
            thread: None,
            editor: None,
            col_widths: vec![],
        };
        let mut scroll = Scroll::default();
        insta::assert_snapshot!(render_text(&view, &mut scroll, 60, 7));
    }
}
