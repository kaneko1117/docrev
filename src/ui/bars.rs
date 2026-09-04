use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::domain::anchor::Anchor;

use super::grid::GridView;
use super::layout::{self, GridLayout};
use super::style::{canvas, chrome};
use super::text::{cell_text, clip, query_line, sanitize};
use super::theme::Palette;

pub struct SearchView {
    pub query: String,
    /// 1-based position among the matches; 0 when there is none.
    pub current: usize,
    pub total: usize,
}

fn column_label(index: u32) -> String {
    Anchor::column_label(index)
}

/// A merged region shows its range and the anchor value; a formula cell shows the formula.
pub(crate) fn draw_formula_bar(p: &Palette, frame: &mut Frame, area: Rect, view: &GridView) {
    let (row, col) = view.cursor;
    let (address, cell) = match view.sheet.merge_at(row, col) {
        Some(merge) => {
            let start = Anchor::cell("", merge.start_row as u32, merge.start_col as u32);
            let end = Anchor::cell("", merge.end_row as u32, merge.end_col as u32);
            (
                format!("{}:{}", start.cell_ref(), end.cell_ref()),
                merge.anchor(),
            )
        }
        None => (
            format!("{}{}", column_label(col as u32), row + 1),
            (row, col),
        ),
    };
    let value = match view.sheet.formula_at(cell.0, cell.1) {
        Some(formula) => format!("={formula}"),
        None => cell_text(view.sheet.cell(cell.0, cell.1)),
    };
    let address = format!(" {address:<7}");
    let used = unicode_width::UnicodeWidthStr::width(address.as_str()) + 2;
    let value = clip(
        &sanitize(&value),
        (area.width as usize).saturating_sub(used),
    );
    let line = Line::from(vec![
        Span::styled(address, canvas(p).add_modifier(Modifier::BOLD)),
        Span::styled("│ ", chrome(p)),
        Span::styled(value, canvas(p)),
    ]);
    frame.render_widget(Paragraph::new(line).style(canvas(p)), area);
}

/// (sheet index, absolute column range).
pub(crate) type TabSpans = Vec<(usize, std::ops::Range<u16>)>;

pub(crate) fn draw_tabs(
    p: &Palette,
    frame: &mut Frame,
    area: Rect,
    view: &GridView,
) -> (TabSpans, Option<u16>, Option<u16>) {
    let names: Vec<&str> = view.tabs.iter().map(|(_, name)| *name).collect();
    let active_tab = view
        .tabs
        .iter()
        .position(|(i, _)| *i == view.active)
        .unwrap_or(0);
    let strip = layout::tab_strip(&names, active_tab, area.width as usize);
    let mut spans = Vec::with_capacity(strip.tabs.len() + 2);
    let mut tab_spans = Vec::with_capacity(strip.tabs.len());
    let mut x = area.x;
    let arrow_left = strip.more_left.then(|| {
        spans.push(Span::styled("‹", chrome(p)));
        x += 1;
        x - 1
    });
    for (tab, label) in &strip.tabs {
        let sheet = view.tabs.get(*tab).map_or(*tab, |(i, _)| *i);
        let style = if sheet == view.active {
            canvas(p).add_modifier(Modifier::BOLD)
        } else {
            chrome(p)
        };
        let width = unicode_width::UnicodeWidthStr::width(label.as_str()) as u16;
        tab_spans.push((sheet, x..x + width));
        x += width;
        spans.push(Span::styled(label.clone(), style));
    }
    let arrow_right = strip.more_right.then(|| {
        spans.push(Span::styled("›", chrome(p)));
        x
    });
    frame.render_widget(Paragraph::new(Line::from(spans)).style(chrome(p)), area);
    (tab_spans, arrow_left, arrow_right)
}

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
    let (row, col) = view.cursor;
    let hint = if view.sheet.workbook_comments_at(row, col).is_empty() {
        hint.to_string()
    } else {
        format!("n:notes  {hint}")
    };
    let hint = match visible_range(grid) {
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

/// `None` when every column is on screen.
/// `None` when every column that can be shown is on screen (frozen ones included).
fn visible_range(grid: &GridLayout) -> Option<String> {
    let showable = grid.col_count - grid.hidden_cols;
    let body: Vec<usize> = grid
        .col_spans
        .iter()
        .map(|(col, _)| *col)
        .filter(|&col| col >= grid.frozen_cols)
        .collect();
    if grid.empty || body.is_empty() || grid.col_spans.len() >= showable {
        return None;
    }
    let first = Anchor::column_label(body[0] as u32);
    let last = Anchor::column_label(body[body.len() - 1] as u32);
    Some(format!("{first}–{last} / {showable}"))
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
            tabs: vec![(0, "wide")],
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
        let text = render_text(&view, &mut Scroll::default(), 80, 8);
        assert!(
            text.contains("/ 30"),
            "the sheet width must be visible:\n{text}"
        );
        assert!(text.contains("A–"), "the range starts at the first column");

        let small = sheet_3x3();
        let mut small_view = view;
        small_view.sheet = &small;
        let text = render_text(&small_view, &mut Scroll::default(), 80, 8);
        assert!(!text.contains(" / "), "no indicator when nothing is hidden");
    }

    #[test]
    fn the_formula_bar_shows_formulas_and_the_grid_keeps_results() {
        use std::collections::HashMap;
        let sheet = Sheet::new(
            "s",
            vec![vec![
                CellValue::Number(5.0),
                CellValue::Number(2.0),
                CellValue::Number(3.0),
            ]],
        )
        .with_formulas(HashMap::from([((0, 0), "SUM(B1:C1)".to_string())]));
        let mut view = GridView {
            sheet: &sheet,
            tabs: vec![(0, "s")],
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
        let text = render_text(&view, &mut Scroll::default(), 46, 6);
        let bar = text.lines().next().unwrap();
        assert!(
            bar.contains("=SUM(B1:C1)"),
            "the bar shows the formula:\n{text}"
        );
        assert!(
            text.lines().nth(2).unwrap().contains('5'),
            "the grid keeps the result:\n{text}"
        );

        view.cursor = (0, 1);
        let text = render_text(&view, &mut Scroll::default(), 46, 6);
        assert!(
            text.lines().next().unwrap().contains('2'),
            "a plain cell keeps showing its value:\n{text}"
        );
    }

    #[test]
    fn a_long_formula_clips_in_the_one_line_bar() {
        use std::collections::HashMap;
        let long = format!("SUM({})", "A1,".repeat(40));
        let sheet = Sheet::new("s", vec![vec![CellValue::Number(1.0)]])
            .with_formulas(HashMap::from([((0, 0), long)]));
        let view = GridView {
            sheet: &sheet,
            tabs: vec![(0, "s")],
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
        let text = render_text(&view, &mut Scroll::default(), 40, 5);
        let bar = text.lines().next().unwrap();
        assert!(bar.contains('…'), "clipped with an ellipsis:\n{bar}");
        assert!(
            unicode_width::UnicodeWidthStr::width(bar) <= 40,
            "never wider than the bar:\n{bar}"
        );
    }

    #[test]
    fn the_hint_offers_n_only_on_cells_with_workbook_comments() {
        use crate::domain::workbook_comment::WorkbookComment;
        let sheet = sheet_3x3().with_workbook_comments(vec![WorkbookComment {
            row: 1,
            col: 1,
            author: "田中".into(),
            body: "メモ".into(),
            resolved: false,
            replies: Vec::new(),
        }]);
        let mut view = GridView {
            sheet: &sheet,
            tabs: vec![(0, "売上")],
            active: 0,
            cursor: (1, 1),
            markers: HashSet::new(),
            notes: HashSet::from([(1, 1)]),
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
        let on_note = render_text(&view, &mut Scroll::default(), 60, 7);
        assert!(on_note.contains("n:notes"), "{on_note}");

        view.cursor = (0, 0);
        let off_note = render_text(&view, &mut Scroll::default(), 60, 7);
        assert!(!off_note.contains("n:notes"), "{off_note}");
    }

    #[test]
    fn the_search_prompt_takes_over_the_status_bar() {
        let sheet = sheet_3x3();
        let mut view = GridView {
            sheet: &sheet,
            tabs: vec![(0, "売上")],
            active: 0,
            cursor: (0, 0),
            markers: HashSet::new(),
            notes: HashSet::new(),
            notes_view: None,
            notice: Some("this notice must yield to the prompt"),
            thread: None,
            editor: None,
            selection: None,
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
        let names = [
            "とても長い名前のシート1",
            "2月度実績データ",
            "3月度実績データ",
            "4月度実績データ",
            "集計",
        ];
        let mut view = GridView {
            sheet: &sheet,
            tabs: names.iter().copied().enumerate().collect(),
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
