//! The sheet-picker dialog: a centered modal over a dimmed screen.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use super::picker::{self, PickerView};
use super::style::{canvas, dialog, selected};
use super::text::sanitize;
use super::theme::Palette;

const PICKER_HINT: &str = " Enter:switch  Esc:cancel ";

/// Centered popup: the query line, a rule, then the candidates windowed
/// around the selection. The grid behind never moves until Enter.
pub(crate) fn draw_picker(p: &Palette, frame: &mut Frame, view: &PickerView) {
    let area = frame.area();
    // a scrim: dim everything behind, so the dialog is unmistakably on top
    frame.render_widget(
        Block::new().style(Style::new().add_modifier(Modifier::DIM)),
        area,
    );
    let width = area
        .width
        .saturating_sub(4)
        .clamp(24, 50)
        .min(area.width.max(1));
    let inner_width = width.saturating_sub(2) as usize;
    // borders + query line + rule take 4 rows; candidates get the rest
    let visible = view
        .items
        .len()
        .clamp(1, area.height.saturating_sub(6).max(1) as usize);
    let height = (visible as u16 + 4).min(area.height);
    let popup = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    };
    let layout = picker::picker_layout(view, inner_width, visible);

    let mut lines = Vec::with_capacity(visible + 2);
    // the input field keeps the canvas white — a light well in the gray
    lines.push(Line::styled(
        picker::query_line(&sanitize(&view.query), inner_width),
        canvas(p),
    ));
    lines.push(Line::styled(
        "─".repeat(inner_width),
        dialog(p).add_modifier(Modifier::DIM),
    ));
    if layout.no_match {
        lines.push(Line::styled(
            "  no match",
            dialog(p).add_modifier(Modifier::DIM),
        ));
    }
    for row in &layout.lines {
        let base = if row.selected { selected(p) } else { dialog(p) };
        let base = if row.active {
            base.add_modifier(Modifier::BOLD)
        } else {
            base
        };
        lines.push(Line::from(vec![
            Span::styled(row.name.clone(), base),
            Span::styled(row.count.clone(), base.fg(p.marker_fg)),
        ]));
    }

    frame.render_widget(Clear, popup);
    // a crisp, undimmed frame — against the scrim it is the one sharp edge
    let widget = Paragraph::new(lines).style(dialog(p)).block(
        Block::bordered()
            .title(Line::styled(
                " Go to sheet ",
                dialog(p).add_modifier(Modifier::BOLD),
            ))
            .title_bottom(PICKER_HINT)
            .title_bottom(Line::from(format!(" {} ", layout.counter)).right_aligned())
            .border_style(dialog(p)),
    );
    frame.render_widget(widget, popup);
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::ui::grid::{GridView, PickerItem, PickerView, Scroll};
    use crate::ui::test_support::{render_text, sheet_3x3};
    use crate::ui::theme::Theme;

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
            search: None,
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
            search: None,
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
            search: None,
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
            search: None,
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
                search: None,
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
