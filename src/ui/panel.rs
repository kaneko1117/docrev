//! The sidebar: the cursor's thread, plus the comment editor — docked at the
//! sidebar's bottom when there is room, overlaying the frame otherwise.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::grid::{EditorKind, EditorView, GridView};
use super::style::{canvas, chrome};
use super::text::{sanitize, wrap};
use super::theme::Palette;

const PANEL_MIN_WIDTH: u16 = 32;
const PANEL_MAX_WIDTH: u16 = 48;
/// Grid columns that must remain visible beside the sidebar.
pub(crate) const GRID_MIN_WIDTH: u16 = 20;
/// Rows the docked editor needs to show a border plus one line of text.
pub(crate) const MIN_DOCKED_EDITOR: u16 = 3;

/// One third of the screen, clamped, and only when the grid keeps its
/// minimum width.
pub(crate) fn panel_width(total: u16, wanted: bool) -> Option<u16> {
    if !wanted {
        return None;
    }
    let width = (total / 3).clamp(PANEL_MIN_WIDTH, PANEL_MAX_WIDTH);
    (total >= width + GRID_MIN_WIDTH).then_some(width)
}

/// The sidebar: the cursor's thread, with the comment editor docked at the
/// bottom while composing so the grid is never covered.
pub(crate) fn draw_panel(
    p: &Palette,
    frame: &mut Frame,
    area: Rect,
    view: &GridView,
    docked: bool,
) {
    let (thread_area, editor_area) = match view.editor.as_ref().filter(|_| docked) {
        Some(editor) => {
            let height = editor_height(editor, editor_inner_width(area.width), area.height);
            let [t, e] =
                Layout::vertical([Constraint::Min(0), Constraint::Length(height)]).areas(area);
            (t, Some(e))
        }
        None => (area, None),
    };

    // wrapped by us, not ratatui: the height must be known so the panel can
    // follow its tail — with the editor docked below (#48), the newest reply
    // is what the user came to read, and it must never hide behind the box
    let inner_width = thread_area.width.saturating_sub(1).max(1) as usize;
    let mut lines = Vec::new();
    if let Some(thread) = view.thread {
        let title = if thread.resolved {
            format!("{} (resolved)", thread.anchor.cell_ref())
        } else {
            thread.anchor.cell_ref()
        };
        lines.push(Line::styled(title, canvas(p).add_modifier(Modifier::BOLD)));
        lines.push(Line::raw(""));
        push_message(p, &mut lines, &thread.author, &thread.body, inner_width);
        for reply in &thread.replies {
            lines.push(Line::raw(""));
            push_message(p, &mut lines, &reply.author, &reply.body, inner_width);
        }
    }
    let scroll = lines.len().saturating_sub(thread_area.height as usize) as u16;
    let panel = Paragraph::new(lines)
        .style(canvas(p))
        .scroll((scroll, 0))
        .block(Block::new().borders(Borders::LEFT).border_style(chrome(p)));
    frame.render_widget(panel, thread_area);

    if let (Some(rect), Some(editor)) = (editor_area, view.editor.as_ref()) {
        draw_editor(p, frame, rect, editor);
    }
}

fn push_message(p: &Palette, lines: &mut Vec<Line>, author: &str, body: &str, width: usize) {
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
        for wrapped in wrap(&sanitize(part), width.saturating_sub(2).max(1)) {
            lines.push(Line::raw(format!("  {wrapped}")));
        }
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
                .title(match editor.kind {
                    EditorKind::Comment => format!(" Comment on {} ", editor.address),
                    EditorKind::Reply => format!(" Reply on {} ", editor.address),
                })
                // the hint rides on the border so it can never be scrolled
                // out of a short box
                .title_bottom(EDITOR_HINT)
                .border_style(chrome(p)),
        );
    frame.render_widget(widget, area);
}

/// Fallback for terminals too narrow for a sidebar.
pub(crate) fn draw_editor_overlay(p: &Palette, frame: &mut Frame, editor: &EditorView) {
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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::domain::anchor::Anchor;
    use crate::domain::comment::CommentThread;
    use crate::domain::sheet::Sheet;
    use crate::ui::grid::{EditorView, GridView, Scroll};
    use crate::ui::test_support::{render_text, sheet_3x3};
    use crate::ui::theme::Theme;

    use super::*;

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
    fn the_panel_shows_the_thread_while_composing() {
        let sheet = sheet_3x3();
        let thread = sample_thread();
        let view = GridView {
            sheet: &sheet,
            sheet_names: vec!["売上"],
            active: 0,
            cursor: (1, 1),
            markers: HashSet::from([(1, 1)]),
            notes: HashSet::new(),
            notes_view: None,
            notice: None,
            thread: Some(&thread),
            editor: Some(EditorView {
                kind: EditorKind::Reply,
                address: "B2".into(),
                buffer: "",
            }),
            selection: None,
            search: None,
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
        };
        let mut scroll = Scroll::default();
        insta::assert_snapshot!(render_text(&view, &mut scroll, 76, 12));
    }

    /// #48: with the editor docked below, the panel follows its tail — the
    /// newest reply is what the user came to read.
    #[test]
    fn the_newest_reply_stays_visible_above_the_editor() {
        let sheet = sheet_3x3();
        let thread = sample_thread();
        for height in [10u16, 12, 24] {
            let view = GridView {
                sheet: &sheet,
                sheet_names: vec!["売上"],
                active: 0,
                cursor: (1, 1),
                markers: HashSet::from([(1, 1)]),
                notes: HashSet::new(),
                notes_view: None,
                notice: None,
                thread: Some(&thread),
                editor: Some(EditorView {
                    kind: EditorKind::Reply,
                    address: "B2".into(),
                    buffer: "one\ntwo\nthree",
                }),
                selection: None,
                search: None,
                picker: None,
                col_widths: vec![],
                theme: Theme::default(),
            };
            let text = render_text(&view, &mut Scroll::default(), 76, height);
            assert!(
                text.contains("150円です"),
                "the latest reply must be readable at height {height}:\n{text}"
            );
        }
    }

    /// #48: the marker alone says a thread is there — moving onto the cell
    /// must not reshape the grid.
    #[test]
    fn a_thread_under_the_cursor_alone_never_moves_the_layout() {
        let sheet = sheet_3x3();
        let thread = sample_thread();
        let mut view = GridView {
            sheet: &sheet,
            sheet_names: vec!["売上"],
            active: 0,
            cursor: (1, 1),
            markers: HashSet::from([(1, 1)]),
            notes: HashSet::new(),
            notes_view: None,
            notice: None,
            thread: Some(&thread),
            editor: None,
            selection: None,
            search: None,
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
        };
        let with_thread = render_text(&view, &mut Scroll::default(), 76, 10);
        assert!(
            with_thread.contains("r:reply"),
            "the hint still points at the thread:\n{with_thread}"
        );
        assert!(
            !with_thread.contains("単価が古い"),
            "the thread body stays closed:\n{with_thread}"
        );

        view.thread = None;
        let without_thread = render_text(&view, &mut Scroll::default(), 76, 10);
        let body = |s: &str| s.rsplit_once('\n').map(|(b, _)| b.to_string()).unwrap();
        assert_eq!(
            body(&with_thread),
            body(&without_thread),
            "everything but the status hint is identical"
        );
    }

    fn composing_view<'a>(sheet: &'a Sheet, buffer: &'a str) -> GridView<'a> {
        GridView {
            sheet,
            sheet_names: vec!["売上"],
            active: 0,
            cursor: (1, 1),
            markers: HashSet::new(),
            notes: HashSet::new(),
            notes_view: None,
            notice: None,
            thread: None,
            editor: Some(EditorView {
                kind: EditorKind::Comment,
                address: "B2".into(),
                buffer,
            }),
            selection: None,
            search: None,
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
            notes: HashSet::new(),
            notes_view: None,
            notice: None,
            thread: None,
            editor: Some(EditorView {
                kind: EditorKind::Comment,
                address: "A1".into(),
                buffer: &long,
            }),
            selection: None,
            search: None,
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
}
