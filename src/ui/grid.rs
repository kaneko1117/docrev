use std::collections::HashSet;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::domain::anchor::Anchor;
use crate::domain::comment::CommentThread;
use crate::domain::sheet::{Rgb, Sheet, TextColor};

use super::layout::{self, GridLayout, LayoutInput, Separator, Viewport};
use super::text::{cell_text, sanitize, wrap};
use super::theme::{Palette, Theme};

pub use super::layout::{DEFAULT_CELL_WIDTH, Scroll};

/// Formula bar + header line + tab bar + status bar.
pub const CHROME_ROWS: u16 = 4;
const PANEL_MIN_WIDTH: u16 = 32;
const PANEL_MAX_WIDTH: u16 = 48;
/// Grid columns that must remain visible beside the sidebar.
const GRID_MIN_WIDTH: u16 = 20;
/// Rows the docked editor needs to show a border plus one line of text.
const MIN_DOCKED_EDITOR: u16 = 3;

fn canvas(p: &Palette) -> Style {
    Style::new().bg(p.canvas_bg).fg(p.text)
}

fn header(p: &Palette) -> Style {
    Style::new().bg(p.header_bg).fg(p.header_fg)
}

fn gridline_style(p: &Palette) -> Style {
    if p.dim_chrome {
        canvas(p).add_modifier(Modifier::DIM)
    } else {
        canvas(p).fg(p.gridline)
    }
}

fn selected(p: &Palette) -> Style {
    if p.reverse_selection {
        return canvas(p).add_modifier(Modifier::REVERSED);
    }
    Style::new().bg(p.selection_bg).fg(p.text)
}

fn chrome(p: &Palette) -> Style {
    if p.dim_chrome {
        canvas(p).add_modifier(Modifier::DIM)
    } else {
        header(p)
    }
}

/// Horizontal gridlines without spending screen rows: a colored underline.
fn ruled(p: &Palette, style: Style) -> Style {
    if p.dim_chrome {
        return style.add_modifier(Modifier::UNDERLINED);
    }
    style
        .add_modifier(Modifier::UNDERLINED)
        .underline_color(p.gridline)
}

/// Canvas painted with the workbook fill when the cell has one — only for
/// palettes whose background the workbook's absolute colors were meant for.
fn filled_canvas(p: &Palette, fill: Option<Rgb>) -> Style {
    match fill.filter(|_| p.paint_workbook_colors) {
        Some(f) => canvas(p).bg(Color::Rgb(f.r, f.g, f.b)),
        None => canvas(p),
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
    pub theme: Theme,
}

pub struct EditorView<'a> {
    pub title: String,
    pub buffer: &'a str,
}

pub fn draw(frame: &mut Frame, view: &GridView, scroll: &mut Scroll) {
    let p = &view.theme.palette();
    let [formula_area, main_area, tabs_area, status_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    // the sidebar carries the thread and, while composing, the editor —
    // so it opens for either one
    let wants_panel = view.thread.is_some() || view.editor.is_some();
    let (grid_area, panel_area) = match panel_width(main_area.width, wants_panel) {
        Some(width) => {
            let [g, p] =
                Layout::horizontal([Constraint::Min(GRID_MIN_WIDTH), Constraint::Length(width)])
                    .areas(main_area);
            (g, Some(p))
        }
        None => (main_area, None),
    };
    draw_formula_bar(p, frame, formula_area, view);
    let grid = layout::grid_layout(
        &LayoutInput {
            sheet: view.sheet,
            cursor: view.cursor,
            markers: &view.markers,
            col_widths: &view.col_widths,
        },
        &Viewport {
            width: grid_area.width as usize,
            rows: grid_area.height.saturating_sub(1) as usize,
        },
        scroll,
    );
    draw_grid(p, frame, grid_area, &grid);
    // the editor docks in the sidebar when there is room for it; otherwise it
    // overlays the whole frame so composing still works on small terminals
    let docked =
        view.editor.is_some() && panel_area.is_some_and(|panel| panel.height >= MIN_DOCKED_EDITOR);
    if let Some(panel) = panel_area {
        draw_panel(p, frame, panel, view, docked);
    }
    if !docked {
        if let Some(editor) = &view.editor {
            draw_editor_overlay(p, frame, editor);
        }
    }
    draw_tabs(p, frame, tabs_area, view);
    draw_status(p, frame, status_area, view, &grid);
}

/// One third of the screen, clamped, and only when the grid keeps its
/// minimum width.
fn panel_width(total: u16, wanted: bool) -> Option<u16> {
    if !wanted {
        return None;
    }
    let width = (total / 3).clamp(PANEL_MIN_WIDTH, PANEL_MAX_WIDTH);
    (total >= width + GRID_MIN_WIDTH).then_some(width)
}

/// Sheets' name box + formula bar: `B2      │ 120`; a merged region shows
/// its range and the anchor value.
fn draw_formula_bar(p: &Palette, frame: &mut Frame, area: Rect, view: &GridView) {
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
            canvas(p).add_modifier(Modifier::BOLD),
        ),
        Span::styled("│ ", chrome(p)),
        Span::styled(value, canvas(p)),
    ]);
    frame.render_widget(Paragraph::new(line).style(canvas(p)), area);
}

/// Thin translator (#36): all layout decisions live in `layout::grid_layout`;
/// this maps the resulting description to `Span`s and nothing else.
fn draw_grid(p: &Palette, frame: &mut Frame, area: Rect, grid: &GridLayout) {
    if grid.empty {
        frame.render_widget(Paragraph::new("(empty sheet)").style(canvas(p)), area);
        return;
    }

    let mut lines = Vec::with_capacity(grid.lines.len() + 1);
    let mut header_line = vec![Span::styled(
        " ".repeat(grid.label_width),
        ruled(p, header(p)),
    )];
    for label in &grid.header {
        header_line.push(Span::styled("│", ruled(p, chrome(p))));
        header_line.push(Span::styled(label.clone(), ruled(p, chrome(p))));
    }
    lines.push(Line::from(header_line));

    for body in &grid.lines {
        let rule = |style: Style| if body.ruled { ruled(p, style) } else { style };
        let mut spans = vec![Span::styled(body.label.clone(), rule(chrome(p)))];
        for slot in &body.slots {
            spans.push(match &slot.separator {
                Separator::Marker { fill } => {
                    Span::styled("●", rule(filled_canvas(p, *fill).fg(p.marker_fg)))
                }
                Separator::Gridline => Span::styled("│", rule(gridline_style(p))),
            });
            let base = if slot.cursor {
                selected(p)
            } else {
                filled_canvas(p, slot.fill)
            };
            let base = match slot.font {
                Some(TextColor::Format(color)) => base.fg(p.format_fg(color)),
                Some(TextColor::Font(rgb)) if p.paint_workbook_colors => {
                    base.fg(Color::Rgb(rgb.r, rgb.g, rgb.b))
                }
                Some(TextColor::Font(_)) => base,
                None => base,
            };
            let style = if slot.ruled { ruled(p, base) } else { base };
            spans.push(Span::styled(slot.text.clone(), style));
        }
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines).style(canvas(p)), area);
}

/// The sidebar: the cursor's thread, with the comment editor docked at the
/// bottom while composing so the grid is never covered.
fn draw_panel(p: &Palette, frame: &mut Frame, area: Rect, view: &GridView, docked: bool) {
    let (thread_area, editor_area) = match view.editor.as_ref().filter(|_| docked) {
        Some(editor) => {
            let height = editor_height(editor, editor_inner_width(area.width), area.height);
            let [t, e] =
                Layout::vertical([Constraint::Min(0), Constraint::Length(height)]).areas(area);
            (t, Some(e))
        }
        None => (area, None),
    };

    let mut lines = Vec::new();
    if let Some(thread) = view.thread {
        let title = if thread.resolved {
            format!("{} (resolved)", thread.anchor.cell_ref())
        } else {
            thread.anchor.cell_ref()
        };
        lines.push(Line::styled(title, canvas(p).add_modifier(Modifier::BOLD)));
        lines.push(Line::raw(""));
        push_message(p, &mut lines, &thread.author, &thread.body);
        for reply in &thread.replies {
            lines.push(Line::raw(""));
            push_message(p, &mut lines, &reply.author, &reply.body);
        }
    }
    let panel = Paragraph::new(lines)
        .style(canvas(p))
        .wrap(Wrap { trim: false })
        .block(Block::new().borders(Borders::LEFT).border_style(chrome(p)));
    frame.render_widget(panel, thread_area);

    if let (Some(rect), Some(editor)) = (editor_area, view.editor.as_ref()) {
        draw_editor(p, frame, rect, editor);
    }
}

fn push_message(p: &Palette, lines: &mut Vec<Line>, author: &str, body: &str) {
    let color = if author == "user" {
        p.user_fg
    } else {
        p.agent_fg
    };
    lines.push(Line::styled(
        format!(" {author}:"),
        Style::new().bg(p.canvas_bg).fg(color),
    ));
    for part in body.split('\n') {
        lines.push(Line::raw(format!("  {}", sanitize(part))));
    }
}

const EDITOR_HINT: &str = " Ctrl+S:save  Esc:cancel ";

/// Text columns inside the editor's border.
fn editor_inner_width(width: u16) -> usize {
    width.saturating_sub(2).max(1) as usize
}

/// The editor's visible lines, wrapped by us rather than by ratatui: the
/// height estimate and the render must agree exactly, and ratatui's `Wrap`
/// breaks on words (a long word or URL would silently need more rows).
/// The last line carries the cursor block.
fn editor_lines(editor: &EditorView, inner_width: usize) -> Vec<String> {
    let logical: Vec<String> = editor.buffer.split('\n').map(sanitize).collect();
    let mut out = Vec::new();
    for (i, line) in logical.iter().enumerate() {
        let text = if i + 1 == logical.len() {
            format!("{line}█")
        } else {
            line.clone()
        };
        out.extend(wrap(&text, inner_width));
    }
    out
}

/// Bounded by what the area can actually give: at most two thirds of the
/// sidebar, and never more than its height.
fn editor_height(editor: &EditorView, inner_width: usize, available: u16) -> u16 {
    if available == 0 {
        return 0;
    }
    let rows = editor_lines(editor, inner_width).len() as u16;
    let cap = (available * 2 / 3).max(3).min(available);
    rows.saturating_add(2).clamp(1, cap)
}

/// `G–L / 30` while columns are off screen.
fn visible_range(grid: &GridLayout) -> Option<String> {
    let visible = grid.visible_cols.len();
    if grid.empty || visible == 0 || visible >= grid.col_count {
        return None;
    }
    let first = Anchor::column_label(grid.visible_cols.start as u32);
    let last = Anchor::column_label((grid.visible_cols.end - 1) as u32);
    Some(format!("{first}–{last} / {}", grid.col_count))
}

fn draw_editor(p: &Palette, frame: &mut Frame, area: Rect, editor: &EditorView) {
    let lines = editor_lines(editor, editor_inner_width(area.width));
    // the cursor lives on the last line, so scrolling to the bottom keeps it
    // visible however small the box gets
    let inner_height = area.height.saturating_sub(2) as usize;
    let scroll = lines.len().saturating_sub(inner_height) as u16;

    frame.render_widget(Clear, area);
    let widget = Paragraph::new(lines.into_iter().map(Line::raw).collect::<Vec<_>>())
        .style(canvas(p))
        .scroll((scroll, 0))
        .block(
            Block::bordered()
                .title(editor.title.clone())
                // the hint rides on the border so it can never be scrolled
                // out of a short box
                .title_bottom(EDITOR_HINT)
                .border_style(chrome(p)),
        );
    frame.render_widget(widget, area);
}

/// Fallback for terminals too narrow for a sidebar.
fn draw_editor_overlay(p: &Palette, frame: &mut Frame, editor: &EditorView) {
    let area = frame.area();
    let width = area.width.saturating_sub(4).clamp(20, 50);
    let height = editor_height(editor, editor_inner_width(width), area.height);
    let popup = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    };
    draw_editor(p, frame, popup, editor);
}

fn draw_tabs(p: &Palette, frame: &mut Frame, area: Rect, view: &GridView) {
    let strip = layout::tab_strip(&view.sheet_names, view.active, area.width as usize);
    let mut spans = Vec::with_capacity(strip.tabs.len() + 2);
    if strip.more_left {
        spans.push(Span::styled("‹", chrome(p)));
    }
    for (i, label) in &strip.tabs {
        let style = if *i == view.active {
            canvas(p).add_modifier(Modifier::BOLD)
        } else {
            chrome(p)
        };
        spans.push(Span::styled(label.clone(), style));
    }
    if strip.more_right {
        spans.push(Span::styled("›", chrome(p)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)).style(chrome(p)), area);
}

/// Hints, notices and — for a sheet wider than the screen — where the view is.
fn draw_status(p: &Palette, frame: &mut Frame, area: Rect, view: &GridView, grid: &GridLayout) {
    let left = match view.notice {
        Some(notice) => format!("⚠ {notice}"),
        None => String::new(),
    };
    let hint = if view.thread.is_some() {
        "r:reply  c:comment  q:quit"
    } else {
        "c:comment  q:quit  Tab:sheet"
    };
    let hint = match visible_range(grid) {
        // only when something is off screen; otherwise it is noise
        Some(range) => format!("{range}  {hint}"),
        None => hint.to_string(),
    };
    let hint = hint.as_str();
    let gap = (area.width as usize).saturating_sub(
        unicode_width::UnicodeWidthStr::width(left.as_str())
            + unicode_width::UnicodeWidthStr::width(hint),
    );
    let line = Line::from(vec![
        Span::styled(left, Style::new().bg(p.header_bg).fg(p.notice_fg)),
        Span::styled(" ".repeat(gap), chrome(p)),
        Span::styled(hint, chrome(p)),
    ]);
    frame.render_widget(Paragraph::new(line).style(chrome(p)), area);
}

#[cfg(test)]
mod tests {
    use crate::domain::cell::CellValue;
    use crate::domain::number_format::FormatColor;
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
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
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
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
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
            Some(Theme::default().palette().format_fg(FormatColor::Red)),
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
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
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
            Some(Theme::default().palette().format_fg(FormatColor::Red)),
            "the [Red] format color beats the blue font"
        );
    }

    #[test]
    fn fills_paint_backgrounds_while_text_wraps() {
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
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
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

        // the long text wraps inside its own column instead of spilling
        // over the filled neighbor
        let text = buffer_text(buffer);
        assert!(
            text.contains("ても長いので"),
            "the text continues on a wrapped line:\n{text}"
        );
        assert!(
            !text.contains('…'),
            "wrapping leaves nothing to clip:\n{text}"
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
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
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
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
        };
        let mut terminal = Terminal::new(TestBackend::new(46, 8)).unwrap();
        terminal
            .draw(|f| draw(f, &view, &mut Scroll::default()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let p = &Theme::default().palette();
        // row 0: formula bar, row 1: column headers, row 2: first data row
        let cursor_cell = buffer.cell((3, 2)).unwrap();
        assert_eq!(cursor_cell.style().bg, Some(p.selection_bg));
        let plain_cell = buffer.cell((3, 3)).unwrap();
        assert_eq!(plain_cell.style().bg, Some(p.canvas_bg));
        let header_cell = buffer.cell((3, 1)).unwrap();
        assert_eq!(header_cell.style().bg, Some(p.header_bg));
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
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
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
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
        };
        let mut scroll = Scroll::default();
        insta::assert_snapshot!(render_text(&view, &mut scroll, 76, 10));
    }

    fn composing_view<'a>(sheet: &'a Sheet, buffer: &'a str) -> GridView<'a> {
        GridView {
            sheet,
            sheet_names: vec!["売上"],
            active: 0,
            cursor: (1, 1),
            markers: HashSet::new(),
            notice: None,
            thread: None,
            editor: Some(EditorView {
                title: " Comment on B2 ".into(),
                buffer,
            }),
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
        }
    }

    #[test]
    fn editor_docks_into_the_sidebar() {
        let sheet = sheet_3x3();
        let view = composing_view(&sheet, "line one\nline two");
        let mut scroll = Scroll::default();
        insta::assert_snapshot!(render_text(&view, &mut scroll, 80, 12));
    }

    #[test]
    fn the_grid_is_never_covered_while_composing() {
        let sheet = sheet_3x3();
        let view = composing_view(&sheet, "typing");
        let text = render_text(&view, &mut Scroll::default(), 80, 12);
        for value in ["項目", "りんご", "みかん"] {
            assert!(text.contains(value), "{value} must stay visible:\n{text}");
        }
        assert!(text.contains('█'), "the editor is on screen too");
    }

    /// The docked editor must keep the cursor and the hint on screen no
    /// matter how the text wraps — long words and URLs included. (The older
    /// test only ran at 50 columns, which falls back to the overlay and so
    /// never exercised this path.)
    #[test]
    fn docked_editor_keeps_cursor_and_hint_visible() {
        let sheet = sheet_3x3();
        for buffer in [
            "see https://example.com/very/long/path/to/the/spec#anchor for details",
            "sixteencharswide sixteencharswide sixteencharswide sixteencharswide",
            "あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほ",
            "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight",
        ] {
            let view = composing_view(&sheet, buffer);
            for (w, h) in [(80u16, 30u16), (80, 12), (200, 30), (52, 8)] {
                let text = render_text(&view, &mut Scroll::default(), w, h);
                assert!(text.contains('█'), "cursor lost at {w}x{h}:\n{text}");
                assert!(
                    text.contains("Ctrl+S:save"),
                    "hint lost at {w}x{h}:\n{text}"
                );
            }
        }
    }

    #[test]
    fn a_short_terminal_falls_back_to_an_overlay() {
        let sheet = sheet_3x3();
        let view = composing_view(&sheet, "typing");
        // the sidebar exists but is too short to host the editor
        for height in [3u16, 4, 5, 6] {
            let text = render_text(&view, &mut Scroll::default(), 80, height);
            assert!(
                text.contains('█'),
                "composing broken at 80x{height}:\n{text}"
            );
        }
    }

    #[test]
    fn a_narrow_terminal_falls_back_to_an_overlay() {
        let sheet = sheet_3x3();
        let view = composing_view(&sheet, "typing");
        // 40 columns cannot host a 32-wide sidebar plus a 20-wide grid
        let text = render_text(&view, &mut Scroll::default(), 40, 12);
        assert!(text.contains('█'), "composing still works:\n{text}");
    }

    #[test]
    fn panel_width_is_a_clamped_third() {
        assert_eq!(
            panel_width(200, true),
            Some(PANEL_MAX_WIDTH),
            "clamped high"
        );
        assert_eq!(panel_width(80, true), Some(PANEL_MIN_WIDTH), "80/3 -> min");
        assert_eq!(panel_width(105, true), Some(35), "a third");
        assert_eq!(panel_width(40, true), None, "no room beside the grid");
        assert_eq!(panel_width(200, false), None, "nothing to show");
    }

    fn wrap_sheet() -> Sheet {
        let long = "あいうえおかきくけこさしすせそ"; // display width 30
        Sheet::new(
            "WR",
            vec![
                vec![CellValue::Text(long.into())],
                vec![CellValue::Text(long.into()), CellValue::Text("X".into())],
                vec![CellValue::Number(1234567890123456.0), CellValue::Empty],
            ],
        )
    }

    #[test]
    fn long_text_wraps_within_its_column() {
        let sheet = wrap_sheet();
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["WR"],
            active: 0,
            cursor: (2, 1),
            markers: HashSet::new(),
            notice: None,
            thread: None,
            editor: None,
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
        };
        let mut scroll = Scroll::default();
        insta::assert_snapshot!(render_text(&view, &mut scroll, 50, 12));
    }

    #[test]
    fn wrapped_lines_carry_no_row_label_and_show_the_tail() {
        let sheet = wrap_sheet();
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["WR"],
            active: 0,
            cursor: (0, 0),
            markers: HashSet::new(),
            notice: None,
            thread: None,
            editor: None,
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
        };
        let text = render_text(&view, &mut Scroll::default(), 50, 12);
        // the full text is visible — wrapped, not clipped (the cursor cell
        // wraps like any other); search bottom-up so the formula bar's
        // full-text preview doesn't match first
        let tail = text
            .lines()
            .rev()
            .find(|l| l.contains("すせそ"))
            .expect("wrapped tail line");
        assert!(
            tail.starts_with("  │"),
            "continuation lines have no row label: {tail:?}"
        );
    }

    #[test]
    fn numbers_never_wrap() {
        let sheet = wrap_sheet();
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["WR"],
            active: 0,
            cursor: (0, 0),
            markers: HashSet::new(),
            notice: None,
            thread: None,
            editor: None,
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
        };
        let text = render_text(&view, &mut Scroll::default(), 50, 12);
        let number_row = text
            .lines()
            .find(|l| l.trim_start().starts_with("3│"))
            .expect("number row");
        assert!(
            number_row.contains('…'),
            "a too-wide number clips on one line instead of wrapping: {number_row:?}"
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
            picker: None,
            col_widths: vec![20, 6, 8],
            theme: Theme::default(),
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
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
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
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
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
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
        };
        let mut terminal = Terminal::new(TestBackend::new(50, 8)).unwrap();
        terminal
            .draw(|f| draw(f, &view, &mut Scroll::default()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        // row 2 is the merged A1:C1 row; column x=3 is inside column A
        let cell = buffer.cell((3, 2)).unwrap();
        assert_eq!(
            cell.style().bg,
            Some(Theme::default().palette().selection_bg)
        );
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
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
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
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
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
    fn wrapping_stays_out_of_a_merge() {
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
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
        };
        let text = render_text(&view, &mut Scroll::default(), 50, 9);
        let grid_row = text
            .lines()
            .find(|l| l.starts_with(" 1│"))
            .expect("data row");
        assert!(
            !grid_row.contains("きくけこ"),
            "the first line holds only the first wrapped segment:\n{text}"
        );
        assert!(
            text.contains("すせそ"),
            "the tail wraps below instead of entering the merge:\n{text}"
        );
    }

    #[test]
    fn the_terminal_theme_paints_no_background_of_its_own() {
        let sheet = sheet_3x3();
        let mut view = GridView {
            sheet: &sheet,
            sheet_names: vec!["売上"],
            active: 0,
            cursor: (1, 1),
            markers: HashSet::new(),
            notice: None,
            thread: None,
            editor: None,
            picker: None,
            col_widths: vec![],
            theme: Theme::Terminal,
        };
        let mut terminal = Terminal::new(TestBackend::new(50, 8)).unwrap();
        terminal
            .draw(|f| draw(f, &view, &mut Scroll::default()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let plain = buffer.cell((10, 3)).unwrap();
        assert_eq!(
            plain.style().bg,
            Some(Color::Reset),
            "the user's terminal background must show through"
        );

        // and the layout is identical either way — only the colors differ
        let terminal_text = buffer_text(buffer);
        view.theme = Theme::Sheets;
        let sheets_text = render_text(&view, &mut Scroll::default(), 50, 8);
        assert_eq!(terminal_text, sheets_text, "themes must not move anything");
    }

    #[test]
    fn a_wide_sheet_shows_where_the_view_is() {
        let sheet = Sheet::new("wide", vec![vec![CellValue::Text("x".into()); 30]]);
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["wide"],
            active: 0,
            cursor: (0, 0),
            markers: HashSet::new(),
            notice: None,
            thread: None,
            editor: None,
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
        };
        let text = render_text(&view, &mut Scroll::default(), 80, 8);
        assert!(
            text.contains("/ 30"),
            "the sheet width must be visible:\n{text}"
        );
        assert!(text.contains("A–"), "the range starts at the first column");

        // a sheet that fits entirely says nothing
        let small = sheet_3x3();
        let mut small_view = view;
        small_view.sheet = &small;
        let text = render_text(&small_view, &mut Scroll::default(), 80, 8);
        assert!(!text.contains(" / "), "no indicator when nothing is hidden");
    }

    #[test]
    fn hidden_sheet_tabs_are_signalled() {
        let sheet = sheet_3x3();
        let names = vec![
            "とても長い名前のシート1",
            "2月度実績データ",
            "3月度実績データ",
            "4月度実績データ",
            "集計",
        ];
        let mut view = GridView {
            sheet: &sheet,
            sheet_names: names.clone(),
            active: 0,
            cursor: (0, 0),
            markers: HashSet::new(),
            notice: None,
            thread: None,
            editor: None,
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
        };
        let text = render_text(&view, &mut Scroll::default(), 60, 8);
        assert!(text.contains('›'), "more sheets to the right:\n{text}");
        assert!(!text.contains('‹'), "none hidden on the left yet");

        view.active = names.len() - 1;
        let text = render_text(&view, &mut Scroll::default(), 60, 8);
        assert!(
            text.contains('‹'),
            "sheets are now hidden to the left:\n{text}"
        );
        assert!(text.contains("集計"), "the active sheet must be visible");
        assert!(!text.contains('›'), "nothing left to the right");
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
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
        };
        let mut scroll = Scroll::default();
        insta::assert_snapshot!(render_text(&view, &mut scroll, 60, 7));
    }

    fn picker(
        query: &str,
        selected: usize,
        total: usize,
        names: &[(&str, usize, bool)],
    ) -> PickerView {
        PickerView {
            query: query.into(),
            selected,
            total,
            items: names
                .iter()
                .map(|(name, count, active)| PickerItem {
                    name: (*name).to_string(),
                    count: *count,
                    active: *active,
                })
                .collect(),
        }
    }

    #[test]
    fn the_sheet_picker_overlays_the_grid() {
        let sheet = sheet_3x3();
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["売上", "経費", "集計"],
            active: 0,
            cursor: (0, 0),
            markers: HashSet::new(),
            notice: None,
            thread: None,
            editor: None,
            picker: Some(picker(
                "",
                0,
                3,
                &[("売上", 2, true), ("経費", 0, false), ("集計", 1, false)],
            )),
            col_widths: vec![],
            theme: Theme::default(),
        };
        let mut scroll = Scroll::default();
        insta::assert_snapshot!(render_text(&view, &mut scroll, 46, 12));
    }

    #[test]
    fn the_picker_reports_no_match() {
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
            picker: Some(picker("zzz", 0, 1, &[])),
            col_widths: vec![],
            theme: Theme::default(),
        };
        let mut scroll = Scroll::default();
        insta::assert_snapshot!(render_text(&view, &mut scroll, 46, 10));
    }

    #[test]
    fn a_tall_candidate_list_keeps_its_bottom_border_above_the_tab_bar() {
        let sheet = sheet_3x3();
        let names: Vec<String> = (1..=32).map(|i| format!("Sheet{i:02}")).collect();
        let items: Vec<(&str, usize, bool)> =
            names.iter().map(|n| (n.as_str(), 0, false)).collect();
        let view = GridView {
            sheet: &sheet,
            sheet_names: names.iter().map(String::as_str).collect(),
            active: 0,
            cursor: (0, 0),
            markers: HashSet::new(),
            notice: None,
            thread: None,
            editor: None,
            picker: Some(picker("", 0, 32, &items)),
            col_widths: vec![],
            theme: Theme::default(),
        };
        let mut scroll = Scroll::default();
        let text = render_text(&view, &mut scroll, 80, 30);
        assert!(text.contains("32/32"), "the counter must survive:\n{text}");
        assert!(
            text.contains("Enter:switch"),
            "the hint must survive:\n{text}"
        );
    }

    #[test]
    fn a_long_query_keeps_its_cursor_on_screen() {
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
            picker: Some(picker(
                "a very long query that outgrows the popup width",
                0,
                1,
                &[],
            )),
            col_widths: vec![],
            theme: Theme::default(),
        };
        let mut scroll = Scroll::default();
        let text = render_text(&view, &mut scroll, 28, 12);
        assert!(text.contains('█'), "the cursor must stay visible:\n{text}");
    }

    #[test]
    fn the_picker_survives_a_tiny_terminal() {
        let sheet = sheet_3x3();
        for (w, h) in [(1, 1), (5, 3), (10, 4), (24, 5), (40, 6)] {
            let view = GridView {
                sheet: &sheet,
                sheet_names: vec!["売上", "経費"],
                active: 0,
                cursor: (0, 0),
                markers: HashSet::new(),
                notice: None,
                thread: None,
                editor: None,
                picker: Some(picker(
                    "とても長い絞り込みの文字列",
                    1,
                    2,
                    &[
                        ("とても長い名前のシートその一", 120, false),
                        ("経費", 0, true),
                    ],
                )),
                col_widths: vec![],
                theme: Theme::default(),
            };
            let mut scroll = Scroll::default();
            render_text(&view, &mut scroll, w, h); // must not panic
        }
    }
}
