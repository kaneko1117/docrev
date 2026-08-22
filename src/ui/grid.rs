//! The screen: partitions the frame and draws the grid body; the chrome
//! (`bars`), the sidebar (`panel`) and the modal (`dialog`) draw the rest.

use std::collections::HashSet;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::domain::comment::CommentThread;
use crate::domain::sheet::{Sheet, TextColor};

use super::layout::{self, GridLayout, LayoutInput, Separator, Viewport};
use super::style::{
    canvas, chrome, filled_canvas, freeze_gridline, freeze_ruled, gridline_style, header,
    note_corner, range_selected, ruled, selected,
};
use super::theme::{Palette, Theme};
use super::{bars, dialog, panel};

pub use super::bars::SearchView;
pub use super::dialog::{NoteView, NotesView};
pub use super::layout::{DEFAULT_CELL_WIDTH, Scroll};
pub use super::picker::{PickerItem, PickerView};

/// Formula bar + header line + tab bar + status bar.
pub const CHROME_ROWS: u16 = 4;

pub struct GridView<'a> {
    pub sheet: &'a Sheet,
    pub sheet_names: Vec<&'a str>,
    pub active: usize,
    pub cursor: (usize, usize),
    /// Cells with an unresolved comment thread, marked with `●`.
    pub markers: HashSet<(usize, usize)>,
    /// Cells with the workbook's own comments — corner-tinted.
    pub notes: HashSet<(usize, usize)>,
    /// The read-only notes dialog, when open.
    pub notes_view: Option<NotesView>,
    pub notice: Option<&'a str>,
    /// Thread under the cursor — drives the status hint, and fills the side
    /// panel while composing.
    pub thread: Option<&'a CommentThread>,
    /// Comment editor state — shown as a popup.
    pub editor: Option<EditorView<'a>>,
    /// Sheet picker state — shown as a centered popup.
    pub picker: Option<PickerView>,
    /// Search prompt state — takes over the status bar.
    pub search: Option<SearchView>,
    /// A drag in progress, highlighted like the cursor.
    pub selection: Option<((usize, usize), (usize, usize))>,
    /// Per-column display widths; missing entries use `DEFAULT_CELL_WIDTH`.
    pub col_widths: Vec<usize>,
    pub theme: Theme,
}

pub struct EditorView<'a> {
    pub title: String,
    pub buffer: &'a str,
}

/// Where things were drawn last frame — the frontend maps mouse coordinates
/// through this.
#[derive(Default, Clone)]
pub struct HitMap {
    /// The grid area; its first line is the column-letter header.
    pub(crate) grid: Rect,
    /// Visible columns as (col, x-range) relative to the grid's left edge.
    pub(crate) col_spans: Vec<(usize, std::ops::Range<usize>)>,
    /// Sheet row per body line, starting under the header line.
    pub(crate) line_rows: Vec<usize>,
    /// The tab bar's row, its clickable spans, and the `‹` / `›` arrows.
    pub(crate) tabs_y: u16,
    pub(crate) tab_spans: Vec<(usize, std::ops::Range<u16>)>,
    pub(crate) arrow_left: Option<u16>,
    pub(crate) arrow_right: Option<u16>,
}

/// What a click landed on.
#[derive(Debug, PartialEq)]
pub enum Hit {
    Cell { row: usize, col: usize },
    Tab(usize),
    PrevTabs,
    NextTabs,
}

impl HitMap {
    pub fn at(&self, x: u16, y: u16) -> Option<Hit> {
        if y == self.tabs_y {
            if Some(x) == self.arrow_left {
                return Some(Hit::PrevTabs);
            }
            if Some(x) == self.arrow_right {
                return Some(Hit::NextTabs);
            }
            let (index, _) = self.tab_spans.iter().find(|(_, s)| s.contains(&x))?;
            return Some(Hit::Tab(*index));
        }
        if !self.grid.contains(ratatui::layout::Position { x, y }) {
            return None;
        }
        let rel_y = (y - self.grid.y) as usize;
        let row = *self.line_rows.get(rel_y.checked_sub(1)?)?;
        let rel_x = (x - self.grid.x) as usize;
        let (col, _) = self.col_spans.iter().find(|(_, s)| s.contains(&rel_x))?;
        Some(Hit::Cell { row, col: *col })
    }
}

pub fn draw(frame: &mut Frame, view: &GridView, scroll: &mut Scroll) -> HitMap {
    let p = &view.theme.palette();
    let [formula_area, main_area, tabs_area, status_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    // the sidebar opens only while composing (#48): the ● marker already
    // says a thread is there, and opening it is the user's decision (`c`) —
    // the grid must not lose a third of its width to a cursor move
    let wants_panel = view.editor.is_some();
    let (grid_area, panel_area) = match panel::panel_width(main_area.width, wants_panel) {
        Some(width) => {
            let [g, p] = Layout::horizontal([
                Constraint::Min(panel::GRID_MIN_WIDTH),
                Constraint::Length(width),
            ])
            .areas(main_area);
            (g, Some(p))
        }
        None => (main_area, None),
    };
    bars::draw_formula_bar(p, frame, formula_area, view);
    let grid = layout::grid_layout(
        &LayoutInput {
            sheet: view.sheet,
            cursor: view.cursor,
            markers: &view.markers,
            notes: &view.notes,
            col_widths: &view.col_widths,
            selection: view.selection,
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
    let docked = view.editor.is_some()
        && panel_area.is_some_and(|panel| panel.height >= panel::MIN_DOCKED_EDITOR);
    if let Some(area) = panel_area {
        panel::draw_panel(p, frame, area, view, docked);
    }
    if !docked {
        if let Some(editor) = &view.editor {
            panel::draw_editor_overlay(p, frame, editor);
        }
    }
    let (tab_spans, arrow_left, arrow_right) = bars::draw_tabs(p, frame, tabs_area, view);
    bars::draw_status(p, frame, status_area, view, &grid);
    // last, so a tall candidate list never loses its bottom border (and the
    // counter riding on it) to the tab bar
    if let Some(picker) = &view.picker {
        dialog::draw_picker(p, frame, picker);
    }
    if let Some(notes) = &view.notes_view {
        dialog::draw_notes(p, frame, notes);
    }
    HitMap {
        grid: grid_area,
        col_spans: grid.col_spans,
        line_rows: grid.lines.iter().map(|l| l.row).collect(),
        tabs_y: tabs_area.y,
        tab_spans,
        arrow_left,
        arrow_right,
    }
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
    for (i, label) in grid.header.iter().enumerate() {
        let separator = if grid.header_boundary == Some(i) {
            ruled(p, freeze_gridline(p))
        } else {
            ruled(p, chrome(p))
        };
        header_line.push(Span::styled("│", separator));
        header_line.push(Span::styled(label.clone(), ruled(p, chrome(p))));
    }
    lines.push(Line::from(header_line));

    for body in &grid.lines {
        let rule = |style: Style| {
            if !body.ruled {
                return style;
            }
            if body.freeze_boundary {
                freeze_ruled(p, style)
            } else {
                ruled(p, style)
            }
        };
        let mut spans = vec![Span::styled(body.label.clone(), rule(chrome(p)))];
        for slot in &body.slots {
            spans.push(match &slot.separator {
                Separator::Marker { fill } => {
                    Span::styled("●", rule(filled_canvas(p, *fill).fg(p.marker_fg)))
                }
                Separator::Gridline if slot.freeze_boundary => {
                    Span::styled("│", rule(freeze_gridline(p)))
                }
                Separator::Gridline => Span::styled("│", rule(gridline_style(p))),
            });
            let base = if slot.cursor {
                selected(p)
            } else if slot.selected {
                range_selected(p)
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
            let style = if slot.ruled {
                if body.freeze_boundary {
                    freeze_ruled(p, base)
                } else {
                    ruled(p, base)
                }
            } else {
                base
            };
            if slot.note {
                // the workbook-comment corner: tint the cell's last character
                let split = slot.text.char_indices().last().map(|(i, _)| i).unwrap_or(0);
                let (head, corner) = slot.text.split_at(split);
                spans.push(Span::styled(head.to_string(), style));
                spans.push(Span::styled(corner.to_string(), note_corner(p)));
            } else {
                spans.push(Span::styled(slot.text.clone(), style));
            }
        }
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines).style(canvas(p)), area);
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Modifier;

    use crate::domain::cell::CellValue;
    use crate::domain::number_format::FormatColor;
    use crate::domain::sheet::Rgb;
    use crate::ui::test_support::{buffer_text, render_text, sheet_3x3};

    use super::*;

    #[test]
    fn renders_grid_tabs_and_status() {
        let sheet = sheet_3x3();
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["売上", "経費"],
            active: 0,
            cursor: (1, 1),
            markers: HashSet::new(),
            notes: HashSet::new(),
            notes_view: None,
            notice: None,
            thread: None,
            editor: None,
            selection: None,
            search: None,
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
            notes: HashSet::new(),
            notes_view: None,
            notice: None,
            thread: None,
            editor: None,
            selection: None,
            search: None,
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
        };
        let mut terminal = Terminal::new(TestBackend::new(46, 7)).unwrap();
        let mut scroll = Scroll::default();
        terminal
            .draw(|f| {
                draw(f, &view, &mut scroll);
            })
            .unwrap();
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
            notes: HashSet::new(),
            notes_view: None,
            notice: None,
            thread: None,
            editor: None,
            selection: None,
            search: None,
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
        };
        let mut terminal = Terminal::new(TestBackend::new(46, 7)).unwrap();
        let mut scroll = Scroll::default();
        terminal
            .draw(|f| {
                draw(f, &view, &mut scroll);
            })
            .unwrap();
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
            notes: HashSet::new(),
            notes_view: None,
            notice: None,
            thread: None,
            editor: None,
            selection: None,
            search: None,
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
        };
        let mut terminal = Terminal::new(TestBackend::new(46, 7)).unwrap();
        let mut scroll = Scroll::default();
        terminal
            .draw(|f| {
                draw(f, &view, &mut scroll);
            })
            .unwrap();
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
            notes: HashSet::new(),
            notes_view: None,
            notice: None,
            thread: None,
            editor: None,
            selection: None,
            search: None,
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
            notes: HashSet::new(),
            notes_view: None,
            notice: None,
            thread: None,
            editor: None,
            selection: None,
            search: None,
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
        };
        let mut terminal = Terminal::new(TestBackend::new(46, 8)).unwrap();
        terminal
            .draw(|f| {
                draw(f, &view, &mut Scroll::default());
            })
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
            notes: HashSet::new(),
            notes_view: None,
            notice: None,
            thread: None,
            editor: None,
            selection: None,
            search: None,
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
        };
        let text = render_text(&view, &mut Scroll::default(), 30, 5);
        assert!(text.contains("(empty sheet)"));
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
            notes: HashSet::new(),
            notes_view: None,
            notice: None,
            thread: None,
            editor: None,
            selection: None,
            search: None,
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
            notes: HashSet::new(),
            notes_view: None,
            notice: None,
            thread: None,
            editor: None,
            selection: None,
            search: None,
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
            notes: HashSet::new(),
            notes_view: None,
            notice: None,
            thread: None,
            editor: None,
            selection: None,
            search: None,
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
            notes: HashSet::new(),
            notes_view: None,
            notice: None,
            thread: None,
            editor: None,
            selection: None,
            search: None,
            picker: None,
            col_widths: vec![20, 6, 8],
            theme: Theme::default(),
        };
        let mut scroll = Scroll::default();
        insta::assert_snapshot!(render_text(&view, &mut scroll, 50, 6));
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
            notes: HashSet::new(),
            notes_view: None,
            notice: None,
            thread: None,
            editor: None,
            selection: None,
            search: None,
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
            notes: HashSet::new(),
            notes_view: None,
            notice: None,
            thread: None,
            editor: None,
            selection: None,
            search: None,
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
        };
        let mut terminal = Terminal::new(TestBackend::new(50, 8)).unwrap();
        terminal
            .draw(|f| {
                draw(f, &view, &mut Scroll::default());
            })
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
            notes: HashSet::new(),
            notes_view: None,
            notice: None,
            thread: None,
            editor: None,
            selection: None,
            search: None,
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
        };
        let mut terminal = Terminal::new(TestBackend::new(50, 8)).unwrap();
        terminal
            .draw(|f| {
                draw(f, &view, &mut Scroll::default());
            })
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
            notes: HashSet::new(),
            notes_view: None,
            notice: None,
            thread: None,
            editor: None,
            selection: None,
            search: None,
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
            notes: HashSet::new(),
            notes_view: None,
            notice: None,
            thread: None,
            editor: None,
            selection: None,
            search: None,
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
            notes: HashSet::new(),
            notes_view: None,
            notice: None,
            thread: None,
            editor: None,
            selection: None,
            search: None,
            picker: None,
            col_widths: vec![],
            theme: Theme::Terminal,
        };
        let mut terminal = Terminal::new(TestBackend::new(50, 8)).unwrap();
        terminal
            .draw(|f| {
                draw(f, &view, &mut Scroll::default());
            })
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

    fn frozen_sheet() -> Sheet {
        let mut rows = vec![vec![
            CellValue::Text("項番".into()),
            CellValue::Text("内容".into()),
            CellValue::Text("結果".into()),
        ]];
        rows.extend((1..60).map(|r| {
            vec![
                CellValue::Text(format!("No{r}")),
                CellValue::Text(format!("手順{r}")),
                CellValue::Text("OK".into()),
            ]
        }));
        Sheet::new("凍結", rows).with_frozen(1, 1)
    }

    #[test]
    fn frozen_panes_pin_headers_while_scrolled() {
        let sheet = frozen_sheet();
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["凍結"],
            active: 0,
            cursor: (50, 2),
            markers: HashSet::new(),
            notes: HashSet::new(),
            notes_view: None,
            notice: None,
            thread: None,
            editor: None,
            selection: None,
            search: None,
            picker: None,
            col_widths: vec![6, 8, 6],
            theme: Theme::default(),
        };
        let mut scroll = Scroll::default();
        insta::assert_snapshot!(render_text(&view, &mut scroll, 40, 9));
    }

    fn hit_map_of(view: &GridView, width: u16, height: u16) -> HitMap {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut hits = HitMap::default();
        terminal
            .draw(|f| {
                hits = draw(f, view, &mut Scroll::default());
            })
            .unwrap();
        hits
    }

    #[test]
    fn the_hit_map_matches_what_was_drawn() {
        let sheet = sheet_3x3();
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["売上", "経費"],
            active: 0,
            cursor: (0, 0),
            markers: HashSet::new(),
            notes: HashSet::new(),
            notes_view: None,
            notice: None,
            thread: None,
            editor: None,
            picker: None,
            selection: None,
            search: None,
            col_widths: vec![],
            theme: Theme::default(),
        };
        let hits = hit_map_of(&view, 46, 8);
        // rows: 0 formula bar, 1 column letters, 2.. data; labels are 2 wide
        assert_eq!(hits.at(3, 2), Some(Hit::Cell { row: 0, col: 0 }));
        assert_eq!(hits.at(16, 3), Some(Hit::Cell { row: 1, col: 1 }));
        assert_eq!(hits.at(3, 0), None, "the formula bar is dead");
        assert_eq!(hits.at(3, 1), None, "the letter header is dead");
        // tab bar sits above the status line: [売上][経費]
        assert_eq!(hits.at(1, 6), Some(Hit::Tab(0)));
        assert_eq!(hits.at(7, 6), Some(Hit::Tab(1)));
        assert_eq!(hits.at(45, 6), None, "the empty tab tail is dead");
    }

    #[test]
    fn the_hit_map_respects_frozen_panes() {
        let sheet = frozen_sheet();
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["凍結"],
            active: 0,
            cursor: (50, 2),
            markers: HashSet::new(),
            notes: HashSet::new(),
            notes_view: None,
            notice: None,
            thread: None,
            editor: None,
            picker: None,
            selection: None,
            search: None,
            col_widths: vec![6, 8, 6],
            theme: Theme::default(),
        };
        let hits = hit_map_of(&view, 40, 9);
        // the pinned header row is line 1 of the grid (screen row 2)
        assert_eq!(hits.at(3, 2), Some(Hit::Cell { row: 0, col: 0 }));
        // the body below it is scrolled near the cursor, not row 1
        let Some(Hit::Cell { row, .. }) = hits.at(3, 3) else {
            panic!("expected a body cell");
        };
        assert!(row > 40, "the body is scrolled to the cursor, got {row}");
    }

    #[test]
    fn the_freeze_boundary_is_emphasized() {
        let sheet = frozen_sheet();
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["凍結"],
            active: 0,
            cursor: (50, 2),
            markers: HashSet::new(),
            notes: HashSet::new(),
            notes_view: None,
            notice: None,
            thread: None,
            editor: None,
            selection: None,
            search: None,
            picker: None,
            col_widths: vec![6, 8, 6],
            theme: Theme::default(),
        };
        let mut terminal = Terminal::new(TestBackend::new(40, 9)).unwrap();
        terminal
            .draw(|f| {
                draw(f, &view, &mut Scroll::default());
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let p = &Theme::default().palette();
        // row 2 is the pinned header row: its underline is the boundary
        let pinned = buffer.cell((1, 2)).unwrap();
        assert_eq!(
            pinned.style().underline_color,
            Some(p.header_fg),
            "the horizontal boundary is darker than a gridline"
        );
        // an ordinary body row keeps the ordinary gridline color
        let body = buffer.cell((1, 3)).unwrap();
        assert_ne!(body.style().underline_color, Some(p.header_fg));
    }

    #[test]
    fn a_workbook_comment_tints_the_cells_corner() {
        let sheet = sheet_3x3();
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["売上"],
            active: 0,
            cursor: (2, 2),
            markers: HashSet::new(),
            notes: HashSet::from([(0, 1)]),
            notes_view: None,
            notice: None,
            thread: None,
            editor: None,
            selection: None,
            search: None,
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
        };
        let mut terminal = Terminal::new(TestBackend::new(46, 7)).unwrap();
        terminal
            .draw(|f| {
                draw(f, &view, &mut Scroll::default());
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let p = &Theme::default().palette();
        // B column spans x=15..28; its last cell (the corner) is x=27, and
        // the first data row is y=2
        let corner = buffer.cell((27, 2)).unwrap();
        assert_eq!(
            corner.style().bg,
            Some(p.notice_fg),
            "the corner carries the tint"
        );
        let neighbour = buffer.cell((26, 2)).unwrap();
        assert_ne!(
            neighbour.style().bg,
            Some(p.notice_fg),
            "one character only — a corner, not a stripe"
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
            notes: HashSet::new(),
            notes_view: None,
            notice: Some("comments unavailable: invalid sidecar"),
            thread: None,
            editor: None,
            selection: None,
            search: None,
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
        };
        let mut scroll = Scroll::default();
        insta::assert_snapshot!(render_text(&view, &mut scroll, 60, 7));
    }
}
