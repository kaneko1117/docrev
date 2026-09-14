use std::collections::HashSet;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::domain::comment::CommentThread;
use crate::domain::text_document::TextDocument;

use super::grid::EditorView;
use super::markdown::{self, Face, Run};
use super::panel;
use super::style::{canvas, chrome, header, selected};
use super::text::{clip, sanitize};
use super::theme::{Palette, Theme};

/// Title bar + status bar.
pub const CHROME_ROWS: u16 = 2;
const EMPTY_PANEL: &str = "(no comment on this line)";

pub struct TextView<'a> {
    pub name: &'a str,
    pub document: &'a TextDocument,
    /// 0-based.
    pub cursor: usize,
    /// Lines with an unresolved thread.
    pub markers: HashSet<usize>,
    pub notice: Option<&'a str>,
    pub thread: Option<&'a CommentThread>,
    pub editor: Option<EditorView<'a>>,
    pub theme: Theme,
}

/// One screen row of the pane: `first` marks the row that carries the line number.
struct Row {
    line: usize,
    first: bool,
    runs: Vec<Run>,
}

pub fn draw(frame: &mut Frame, view: &TextView, top: &mut usize) {
    let p = &view.theme.palette();
    let [title_area, main_area, status_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let (pane_area, panel_area) = match panel::panel_width(main_area.width, true) {
        Some(width) => {
            let [t, p] = Layout::horizontal([
                Constraint::Min(panel::GRID_MIN_WIDTH),
                Constraint::Length(width),
            ])
            .areas(main_area);
            (t, Some(p))
        }
        None => (main_area, None),
    };
    draw_title(p, frame, title_area, view);
    draw_pane(p, frame, pane_area, view, top);
    let docked = view.editor.is_some()
        && panel_area.is_some_and(|panel| panel.height >= panel::MIN_DOCKED_EDITOR);
    if let Some(area) = panel_area {
        let empty = view.thread.is_none().then_some(EMPTY_PANEL);
        panel::draw_panel(
            p,
            frame,
            area,
            view.thread,
            view.editor.as_ref(),
            docked,
            empty,
        );
    }
    if !docked && let Some(editor) = &view.editor {
        panel::draw_editor_overlay(p, frame, editor);
    }
    draw_status(p, frame, status_area, view);
}

/// The position always fits; a long name is clipped to make room.
fn draw_title(p: &Palette, frame: &mut Frame, area: Rect, view: &TextView) {
    let position = if view.document.is_empty() {
        String::new()
    } else {
        format!("line {}/{} ", view.cursor + 1, view.document.len())
    };
    let position_width = unicode_width::UnicodeWidthStr::width(position.as_str());
    let name = clip(
        &format!(" {}", sanitize(view.name)),
        (area.width as usize).saturating_sub(position_width),
    );
    let gap = (area.width as usize)
        .saturating_sub(unicode_width::UnicodeWidthStr::width(name.as_str()) + position_width);
    let line = Line::from(vec![
        Span::styled(name, chrome(p).add_modifier(Modifier::BOLD)),
        Span::styled(" ".repeat(gap), chrome(p)),
        Span::styled(position, chrome(p)),
    ]);
    frame.render_widget(Paragraph::new(line).style(chrome(p)), area);
}

fn draw_status(p: &Palette, frame: &mut Frame, area: Rect, view: &TextView) {
    let left = match view.notice {
        Some(notice) => format!("⚠ {notice}"),
        None => String::new(),
    };
    let hint = "q:quit";
    let gap = (area.width as usize).saturating_sub(
        unicode_width::UnicodeWidthStr::width(left.as_str())
            + unicode_width::UnicodeWidthStr::width(hint),
    );
    let line = Line::from(vec![
        Span::styled(left, chrome(p).fg(p.notice_fg)),
        Span::styled(" ".repeat(gap), chrome(p)),
        Span::styled(hint, chrome(p)),
    ]);
    frame.render_widget(Paragraph::new(line).style(chrome(p)), area);
}

fn draw_pane(p: &Palette, frame: &mut Frame, area: Rect, view: &TextView, top: &mut usize) {
    if view.document.is_empty() {
        frame.render_widget(Paragraph::new("(empty file)").style(canvas(p)), area);
        return;
    }
    let number_width = view.document.len().to_string().len();
    // " ● " + number + " │ "
    let gutter_width = number_width + 6;
    let text_width = (area.width as usize).saturating_sub(gutter_width).max(1);
    let rows = layout(
        view.document,
        text_width,
        area.height as usize,
        view.cursor,
        top,
    );
    let lines: Vec<Line> = rows
        .iter()
        .map(|row| {
            let on_cursor = row.line == view.cursor;
            let (gutter_style, text_style) = if on_cursor {
                (selected(p), selected(p))
            } else {
                (header(p), canvas(p))
            };
            let marker = if row.first && view.markers.contains(&row.line) {
                "●"
            } else {
                " "
            };
            let number = if row.first {
                format!("{:>number_width$}", row.line + 1)
            } else {
                " ".repeat(number_width)
            };
            let mut spans = vec![
                Span::styled(" ", gutter_style),
                Span::styled(marker, gutter_style.fg(p.marker_fg)),
                Span::styled(format!(" {number} │ "), gutter_style),
            ];
            let runs = &row.runs;
            let width: usize = runs
                .iter()
                .map(|(text, _)| unicode_width::UnicodeWidthStr::width(text.as_str()))
                .sum();
            spans.extend(
                runs.iter()
                    .map(|(text, face)| Span::styled(text.clone(), faced(p, text_style, *face))),
            );
            let padding = text_width.saturating_sub(width);
            spans.push(Span::styled(" ".repeat(padding), text_style));
            Line::from(spans)
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).style(canvas(p)), area);
}

/// The face adds to the row's base style, so the cursor row keeps its background.
fn faced(p: &Palette, base: Style, face: Face) -> Style {
    match face {
        Face::Plain => base,
        Face::Heading(1) => base
            .fg(p.heading_fg)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        Face::Heading(_) => base.fg(p.heading_fg).add_modifier(Modifier::BOLD),
        Face::Bold => base.add_modifier(Modifier::BOLD),
        Face::Italic => base.add_modifier(Modifier::ITALIC),
        Face::Code | Face::CodeBlock => base.bg(p.header_bg),
        Face::ListMarker => base.fg(p.marker_fg),
        Face::Link => base.fg(p.user_fg).add_modifier(Modifier::UNDERLINED),
        Face::Quote | Face::FrontMatter => base.add_modifier(Modifier::DIM),
        Face::Strike => base.add_modifier(Modifier::CROSSED_OUT),
        Face::Rule | Face::TableEdge => {
            if p.dim_chrome {
                base.add_modifier(Modifier::DIM)
            } else {
                base.fg(p.gridline)
            }
        }
    }
}

/// Rows from `top`, with `top` moved so every row of the cursor line is on screen and the pane
/// does not end early while lines above `top` would still fit.
fn layout(
    document: &TextDocument,
    text_width: usize,
    height: usize,
    cursor: usize,
    top: &mut usize,
) -> Vec<Row> {
    let len = document.len();
    let blocks = markdown::blocks(document.lines().iter().map(String::as_str));
    let wrapped = |line: usize| {
        let text = sanitize(document.line(line).unwrap_or_default());
        let runs = blocks.render(&text, line);
        // a thematic break spans the pane on one row, whatever its source length
        let runs = if matches!(runs.as_slice(), [(_, Face::Rule)]) {
            vec![("─".repeat(text_width), Face::Rule)]
        } else {
            runs
        };
        wrap_runs(runs, text_width)
    };
    let rows_in = |line: usize| wrapped(line).len();
    let cursor = cursor.min(len.saturating_sub(1));
    // the lowest top that still shows the whole cursor line
    let mut lowest = cursor;
    let mut used = rows_in(cursor);
    while lowest > 0 && used + rows_in(lowest - 1) <= height {
        lowest -= 1;
        used += rows_in(lowest);
    }
    *top = (*top).clamp(lowest, cursor);
    // after a shrink, lines above may fit again: pull the window up until the pane is full
    let mut filled = 0;
    for line in *top..len {
        filled += rows_in(line);
        if filled >= height {
            break;
        }
    }
    while *top > 0 && filled + rows_in(*top - 1) <= height {
        *top -= 1;
        filled += rows_in(*top);
    }
    let mut rows = Vec::new();
    for line in *top..len {
        if rows.len() >= height {
            break;
        }
        rows.extend(wrapped(line).into_iter().enumerate().map(|(i, runs)| Row {
            line,
            first: i == 0,
            runs,
        }));
    }
    rows.truncate(height);
    rows
}

/// Breaks runs into rows of at most `width` cells; a char wider than `width` still gets a row so
/// the loop makes progress, and an empty line is one empty row.
fn wrap_runs(runs: Vec<Run>, width: usize) -> Vec<Vec<Run>> {
    let mut rows: Vec<Vec<Run>> = Vec::new();
    let mut row: Vec<Run> = Vec::new();
    let mut used = 0;
    for (text, face) in runs {
        let mut piece = String::new();
        for ch in text.chars() {
            let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            if used + w > width.max(1) && used > 0 {
                if !piece.is_empty() {
                    row.push((std::mem::take(&mut piece), face));
                }
                rows.push(std::mem::take(&mut row));
                used = 0;
            }
            piece.push(ch);
            used += w;
        }
        if !piece.is_empty() {
            row.push((piece, face));
        }
    }
    rows.push(row);
    rows
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::domain::anchor::Anchor;
    use crate::domain::comment::{CommentThread, Reply};
    use crate::ui::test_support::buffer_text;

    use super::*;

    fn render(view: &TextView, top: &mut usize, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| draw(f, view, top)).unwrap();
        buffer_text(terminal.backend().buffer())
    }

    fn view<'a>(document: &'a TextDocument, cursor: usize) -> TextView<'a> {
        TextView {
            name: "README.md",
            document,
            cursor,
            markers: HashSet::new(),
            notice: None,
            thread: None,
            editor: None,
            theme: Theme::Sheets,
        }
    }

    fn thread(line: u32) -> CommentThread {
        CommentThread {
            id: "t".into(),
            anchor: Anchor::line(line),
            author: "user".into(),
            body: "brew tap comes first".into(),
            created_at: "".into(),
            resolved: false,
            replies: vec![Reply {
                id: "r".into(),
                author: "claude".into(),
                body: "added a step".into(),
                created_at: "".into(),
            }],
        }
    }

    #[test]
    fn renders_title_gutter_markers_cursor_and_the_panel() {
        let document =
            TextDocument::new("# docrev\n\n## Install\nbrew install docrev\n\nCheck it works:\n");
        let t = thread(3);
        let mut v = view(&document, 3);
        v.markers = HashSet::from([3]);
        v.thread = Some(&t);
        v.notice = Some("comments unavailable: boom");
        let mut top = 0;
        insta::assert_snapshot!(render(&v, &mut top, 70, 9));
    }

    #[test]
    fn markdown_is_rendered_line_by_line_with_its_markers_hidden() {
        let document = TextDocument::new(
            "# Title\n- run `docrev` **now**\n```\nlet **raw** = 1;\n```\n> see [docs](https://x)\n",
        );
        let v = view(&document, 5);
        let mut terminal = Terminal::new(TestBackend::new(60, 8)).unwrap();
        terminal.draw(|f| draw(f, &v, &mut 0)).unwrap();
        let buffer = terminal.backend().buffer();
        insta::assert_snapshot!(buffer_text(buffer));
        let cell = |x: u16, y: u16| buffer.cell((x, y)).unwrap();
        // " ● 1 │ " is 7 cells wide, so text starts at x = 7
        assert!(
            cell(7, 1).modifier.contains(Modifier::BOLD),
            "heading is bold"
        );
        assert_eq!(cell(7, 1).fg, Theme::Sheets.palette().heading_fg);
        assert_eq!(cell(7, 2).symbol(), "•");
        assert_eq!(
            cell(13, 2).bg,
            Theme::Sheets.palette().header_bg,
            "inline code"
        );
        assert!(cell(20, 2).modifier.contains(Modifier::BOLD), "**now**");
        assert_eq!(cell(7, 4).symbol(), "l", "fenced code keeps its markers");
        assert_eq!(cell(11, 4).symbol(), "*");
        assert_eq!(
            cell(7, 4).bg,
            Theme::Sheets.palette().header_bg,
            "code block"
        );
        assert!(
            cell(13, 6).modifier.contains(Modifier::UNDERLINED),
            "link label"
        );
        assert_eq!(cell(7, 6).symbol(), ">", "quote mark kept");
    }

    #[test]
    fn front_matter_tasks_tables_and_rules_are_drawn() {
        let document = TextDocument::new(
            "---\ntitle: notes\n---\n# Plan\n- [x] ship it\n- [ ] write docs\n\n| item | qty |\n|-------|-----|\n| apple | 3   |\n\n---\n\n~~dropped~~ and ![logo](l.png)\n",
        );
        let v = view(&document, 0);
        let mut terminal = Terminal::new(TestBackend::new(80, 18)).unwrap();
        terminal.draw(|f| draw(f, &v, &mut 0)).unwrap();
        insta::assert_snapshot!(buffer_text(terminal.backend().buffer()));
    }

    #[test]
    fn a_line_without_a_thread_shows_an_empty_panel() {
        let document = TextDocument::new("one\ntwo\n");
        let v = view(&document, 1);
        let out = render(&v, &mut 0, 70, 5);
        assert!(out.contains(EMPTY_PANEL), "{out}");
        assert!(out.contains("line 2/2"), "{out}");
    }

    #[test]
    fn long_lines_wrap_without_a_number_and_the_cursor_row_stays_on_screen() {
        let text = (1..=12)
            .map(|i| {
                if i == 5 {
                    "x".repeat(40)
                } else {
                    format!("line {i}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let document = TextDocument::new(&text);
        let mut top = 0;
        let out = render(&view(&document, 4), &mut top, 40, 7);
        insta::assert_snapshot!(out);
        assert_eq!(top, 1, "line 1 scrolls off so both rows of line 5 fit");
        let out = render(&view(&document, 0), &mut top, 40, 7);
        assert_eq!(top, 0, "scrolls back up when the cursor is above the top");
        assert!(out.contains(" 1 │ line 1"), "{out}");
    }

    #[test]
    fn narrow_terminals_drop_the_panel_and_an_empty_file_says_so() {
        let document = TextDocument::new("only\n");
        let out = render(&view(&document, 0), &mut 0, 40, 4);
        assert!(!out.contains(EMPTY_PANEL), "{out}");
        assert!(out.contains(" 1 │ only"), "{out}");
        let empty = TextDocument::new("");
        let out = render(&view(&empty, 0), &mut 0, 70, 4);
        assert!(
            out.contains("(empty file)") && !out.contains("line "),
            "{out}"
        );
    }

    #[test]
    fn a_long_name_is_clipped_so_the_position_survives() {
        let document = TextDocument::new("one\n");
        let mut v = view(&document, 0);
        v.name = "日本語ファイル名がとても長い場合のテスト.md";
        let out = render(&v, &mut 0, 40, 3);
        let title = out.lines().next().unwrap_or("");
        assert!(title.ends_with("line 1/1"), "{title:?}");
        assert!(title.contains('…'), "{title:?}");
    }

    #[test]
    fn a_far_jump_costs_only_the_visible_lines() {
        let text = (0..20_000)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let document = TextDocument::new(&text);
        let mut top = 0;
        let started = std::time::Instant::now();
        let rows = layout(&document, 30, 10, 19_999, &mut top);
        assert!(started.elapsed() < std::time::Duration::from_millis(500));
        assert_eq!(top, 19_990);
        assert_eq!(rows.len(), 10);
    }

    #[test]
    fn a_shrunk_file_pulls_the_window_up_to_fill_the_pane() {
        let text = (0..10)
            .map(|i| format!("l{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let document = TextDocument::new(&text);
        let mut top = 90;
        let rows = layout(&document, 30, 20, 9, &mut top);
        assert_eq!(top, 0, "everything fits, so nothing stays hidden above");
        assert_eq!(rows.len(), 10);
        let mut top = 90;
        let rows = layout(&document, 30, 4, 9, &mut top);
        assert_eq!(top, 6, "the last four lines fill the pane");
        assert_eq!(rows.len(), 4);
    }

    #[test]
    fn a_long_rule_stays_on_one_row() {
        let document = TextDocument::new(&format!("before\n{}\nafter\n", "-".repeat(50)));
        let rows = layout(&document, 33, 10, 0, &mut 0);
        assert_eq!(rows.iter().filter(|r| r.line == 1).count(), 1);
        let narrow = layout(&TextDocument::new("---\n"), 1, 5, 0, &mut 0);
        assert_eq!(
            narrow.len(),
            1,
            "a one-cell pane still draws the rule on one row"
        );
    }

    #[test]
    fn layout_keeps_the_cursor_line_whole_when_it_wraps() {
        let document = TextDocument::new("a\nb\nccccccccccdddddddddd\ne\n");
        let mut top = 0;
        let rows = layout(&document, 10, 3, 2, &mut top);
        assert_eq!(top, 1, "line 1 scrolls off so both rows of line 3 fit");
        let shape: Vec<(usize, bool, String)> = rows
            .iter()
            .map(|r| {
                (
                    r.line,
                    r.first,
                    r.runs.iter().map(|(t, _)| t.as_str()).collect(),
                )
            })
            .collect();
        assert_eq!(
            shape,
            vec![
                (1, true, "b".to_string()),
                (2, true, "cccccccccc".to_string()),
                (2, false, "dddddddddd".to_string())
            ]
        );
    }
}
