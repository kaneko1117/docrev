//! The one-line chrome around the grid: formula bar, tab strip, status bar.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::domain::anchor::Anchor;

use super::grid::GridView;
use super::layout::{self, GridLayout};
use super::style::{canvas, chrome};
use super::text::{cell_text, query_line, sanitize};
use super::theme::Palette;

/// Search prompt state — takes over the status bar while searching.
pub struct SearchView {
    pub query: String,
    /// 1-based position among the matches; 0 when there is none.
    pub current: usize,
    pub total: usize,
}

fn column_label(index: u32) -> String {
    Anchor::column_label(index)
}

/// Sheets' name box + formula bar: `B2      │ 120`; a merged region shows
/// its range and the anchor value.
pub(crate) fn draw_formula_bar(p: &Palette, frame: &mut Frame, area: Rect, view: &GridView) {
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

pub(crate) fn draw_tabs(p: &Palette, frame: &mut Frame, area: Rect, view: &GridView) {
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
pub(crate) fn draw_status(
    p: &Palette,
    frame: &mut Frame,
    area: Rect,
    view: &GridView,
    grid: &GridLayout,
) {
    if let Some(search) = &view.search {
        return draw_search(p, frame, area, search);
    }
    let left = match view.notice {
        Some(notice) => format!("⚠ {notice}"),
        None => String::new(),
    };
    let hint = if view.thread.is_some() {
        "r:reply  c:comment  q:quit"
    } else {
        "c:comment  q:quit  ^G:sheet  ^F:find"
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

/// `Find: 合計█            3/17` — the input well is white like the grid so
/// it reads as a place to type; `0/0` turns warning-colored when a non-empty
/// query finds nothing.
fn draw_search(p: &Palette, frame: &mut Frame, area: Rect, search: &SearchView) {
    let label = " Find: ";
    let counter = format!(" {}/{} ", search.current, search.total);
    let width = |s: &str| unicode_width::UnicodeWidthStr::width(s);
    let field = (area.width as usize).saturating_sub(width(label) + width(&counter));
    let query = query_line(&sanitize(&search.query), field);
    let gap = field.saturating_sub(width(&query));
    let counter_style = if search.total == 0 && !search.query.is_empty() {
        Style::new().bg(p.header_bg).fg(p.notice_fg)
    } else {
        chrome(p)
    };
    let line = Line::from(vec![
        Span::styled(label, chrome(p)),
        Span::styled(query, canvas(p)),
        Span::styled(" ".repeat(gap), canvas(p)),
        Span::styled(counter, counter_style),
    ]);
    frame.render_widget(Paragraph::new(line).style(chrome(p)), area);
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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::domain::cell::CellValue;
    use crate::domain::sheet::Sheet;
    use crate::ui::grid::{GridView, Scroll};
    use crate::ui::test_support::{render_text, sheet_3x3};
    use crate::ui::theme::Theme;

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
            search: None,
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
    fn the_search_prompt_takes_over_the_status_bar() {
        let sheet = sheet_3x3();
        let mut view = GridView {
            sheet: &sheet,
            sheet_names: vec!["売上"],
            active: 0,
            cursor: (0, 0),
            markers: HashSet::new(),
            notice: Some("this notice must yield to the prompt"),
            thread: None,
            editor: None,
            search: Some(super::SearchView {
                query: "合計".into(),
                current: 3,
                total: 17,
            }),
            picker: None,
            col_widths: vec![],
            theme: Theme::default(),
        };
        let text = render_text(&view, &mut Scroll::default(), 46, 7);
        assert!(text.contains("Find: 合計█"), "prompt with cursor:\n{text}");
        assert!(text.contains("3/17"), "counter:\n{text}");
        assert!(!text.contains("⚠"), "the notice waits its turn:\n{text}");

        view.search = Some(super::SearchView {
            query: "zzz".into(),
            current: 0,
            total: 0,
        });
        let text = render_text(&view, &mut Scroll::default(), 46, 7);
        assert!(text.contains("0/0"), "no-match counter:\n{text}");
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
            search: None,
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
}
