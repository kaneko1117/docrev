use crate::domain::sheet::Sheet;

use super::matching::{contains_folded, fold};
use super::{Event, Mode, Viewer};

/// `current` is 1-based; 0 means no matches.
pub struct SearchState<'a> {
    pub query: &'a str,
    pub current: usize,
    pub total: usize,
}

/// Row-major, over the displayed text. A merged region matches once, on its
/// anchor; an empty query matches nothing.
fn matches_in(sheet: &Sheet, query: &str) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return Vec::new();
    }
    let needle = fold(query);
    // indexed up front: a per-cell `merge_at` froze merge-heavy sheets;
    // clamped to existing cells so a whole-sheet merge cannot balloon it
    let mut covered = std::collections::HashSet::new();
    for m in sheet.merges() {
        let anchor = m.anchor();
        for r in m.start_row..=m.end_row.min(sheet.row_count().saturating_sub(1)) {
            for c in m.start_col..sheet.row_len(r).min(m.end_col + 1) {
                if (r, c) != anchor {
                    covered.insert((r, c));
                }
            }
        }
    }
    let mut out = Vec::new();
    for row in 0..sheet.row_count() {
        // `row_len`, not `col_count`: one stray value in the last column must
        // not widen every row's scan
        for col in 0..sheet.row_len(row) {
            if covered.contains(&(row, col)) {
                continue;
            }
            let text = sheet.cell(row, col).display_text();
            if !text.is_empty() && contains_folded(&text, &needle) {
                out.push((row, col));
            }
        }
    }
    out
}

/// Row-major, wrapping to the start.
fn first_at_or_after(matches: &[(usize, usize)], origin: (usize, usize)) -> usize {
    matches
        .iter()
        .position(|&m| m >= origin)
        .unwrap_or_default()
}

impl Viewer {
    pub fn search_state(&self) -> Option<SearchState<'_>> {
        let Mode::Search {
            query,
            matches,
            index,
            ..
        } = &self.mode
        else {
            return None;
        };
        Some(SearchState {
            query,
            current: if matches.is_empty() { 0 } else { index + 1 },
            total: matches.len(),
        })
    }

    pub(super) fn apply_search(&mut self, event: Event) {
        let Mode::Search { origin, .. } = &self.mode else {
            return;
        };
        // matches carry the merge anchor, so the cursor must compare as one
        let scan_origin = match self.sheets.get(self.active).merge_at(origin.0, origin.1) {
            Some(merge) => merge.anchor(),
            None => *origin,
        };
        let sheet_matches = |query: &str| matches_in(self.sheets.get(self.active), query);
        let Mode::Search {
            query,
            origin,
            matches,
            index,
        } = &mut self.mode
        else {
            return;
        };
        let origin = *origin;
        match event {
            Event::Insert(c) => {
                query.push(c);
                *matches = sheet_matches(query);
                *index = first_at_or_after(matches, scan_origin);
            }
            Event::Backspace => {
                if query.pop().is_some() {
                    *matches = sheet_matches(query);
                    *index = first_at_or_after(matches, scan_origin);
                }
            }
            Event::Move { rows, .. } => {
                if !matches.is_empty() {
                    let len = matches.len();
                    *index = if rows >= 0 {
                        (*index + 1) % len
                    } else {
                        (*index + len - 1) % len
                    };
                }
            }
            Event::Submit => {
                self.mode = Mode::Grid;
                return;
            }
            Event::CancelEdit => {
                self.set_cursor(origin);
                self.mode = Mode::Grid;
                return;
            }
            Event::Quit => {
                self.quit = true;
                return;
            }
            _ => return,
        }
        let target = match &self.mode {
            Mode::Search { matches, index, .. } => matches.get(*index).copied().unwrap_or(origin),
            _ => return,
        };
        self.set_cursor(target);
    }

    pub(super) fn set_cursor(&mut self, position: (usize, usize)) {
        if let Some(cursor) = self.cursors.get_mut(self.active) {
            *cursor = position;
        }
    }

    /// After a reload: matches are recomputed, the origin clamped, the cursor
    /// deliberately left where it is.
    pub(super) fn refresh_search(&mut self) {
        let sheet = self.sheets.get(self.active);
        let (max_row, max_col) = (
            sheet.row_count().saturating_sub(1),
            sheet.col_count().saturating_sub(1),
        );
        let new_matches = match &self.mode {
            Mode::Search { query, .. } => matches_in(sheet, query),
            _ => return,
        };
        let Mode::Search {
            origin,
            matches,
            index,
            ..
        } = &mut self.mode
        else {
            return;
        };
        origin.0 = origin.0.min(max_row);
        origin.1 = origin.1.min(max_col);
        *index = first_at_or_after(&new_matches, *origin);
        *matches = new_matches;
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::cell::CellValue;
    use crate::domain::document::Document;
    use crate::domain::sheet::{MergedRange, Sheet};

    use super::super::test_support::{NullStore, SharedSource, type_text, viewer_on, viewer_with};
    use super::*;

    fn text(s: &str) -> CellValue {
        CellValue::Text(s.into())
    }

    fn search_sheet() -> Sheet {
        Sheet::new(
            "s",
            vec![
                vec![text("項目"), text("単価"), text("合計")],
                vec![text("りんご"), CellValue::Number(120.0), text("小計")],
                vec![text("みかん"), CellValue::Number(80.0), text("合計欄")],
            ],
        )
    }

    fn search_viewer(sheet: Sheet) -> Viewer {
        Viewer::from_document(
            Document::new(vec![sheet]),
            Vec::new(),
            None,
            None,
            Box::new(NullStore),
        )
        .unwrap()
    }

    #[test]
    fn matches_run_over_displayed_text_in_row_major_order() {
        let sheet = search_sheet();
        assert_eq!(matches_in(&sheet, "合計"), vec![(0, 2), (2, 2)]);
        assert_eq!(matches_in(&sheet, "120"), vec![(1, 1)], "numbers match");
        assert_eq!(matches_in(&sheet, ""), Vec::<(usize, usize)>::new());
        assert_eq!(matches_in(&sheet, "zzz"), Vec::<(usize, usize)>::new());
    }

    #[test]
    fn matching_folds_case_and_full_width() {
        let sheet = Sheet::new("s", vec![vec![text("IT-01 Login")]]);
        assert_eq!(matches_in(&sheet, "ｉｔ－01"), vec![(0, 0)]);
        assert_eq!(matches_in(&sheet, "login"), vec![(0, 0)]);
    }

    #[test]
    fn a_formatted_number_matches_what_is_on_screen() {
        let sheet = Sheet::new(
            "s",
            vec![vec![CellValue::FormattedNumber {
                value: -1234.0,
                text: "▲1,234".into(),
            }]],
        );
        assert_eq!(matches_in(&sheet, "1,234"), vec![(0, 0)]);
        assert_eq!(
            matches_in(&sheet, "-1234"),
            Vec::<(usize, usize)>::new(),
            "the raw value is not what the user sees"
        );
    }

    #[test]
    fn a_merged_region_matches_once_on_its_anchor() {
        let sheet = Sheet::new(
            "s",
            vec![vec![text("題名"), CellValue::Empty, text("題名")]],
        )
        .with_merges(vec![MergedRange {
            start_row: 0,
            start_col: 0,
            end_row: 0,
            end_col: 1,
        }]);
        assert_eq!(matches_in(&sheet, "題名"), vec![(0, 0), (0, 2)]);
    }

    #[test]
    fn a_stray_far_right_cell_is_still_found() {
        let mut wide = vec![CellValue::Empty; 16384];
        wide[16383] = text("迷子");
        let sheet = Sheet::new("s", vec![vec![text("a")], wide, vec![text("迷子ではない")]]);
        assert_eq!(matches_in(&sheet, "迷子"), vec![(1, 16383), (2, 0)]);
    }

    #[test]
    fn searching_from_inside_a_merge_finds_the_match_under_the_cursor() {
        let sheet = Sheet::new(
            "s",
            vec![
                vec![text("合計"), CellValue::Empty],
                vec![text("合計その2"), CellValue::Empty],
            ],
        )
        .with_merges(vec![MergedRange {
            start_row: 0,
            start_col: 0,
            end_row: 0,
            end_col: 1,
        }]);
        let mut v = search_viewer(sheet);
        v.apply(Event::Move { rows: 0, cols: 1 }); // covered by the merge
        v.apply(Event::OpenSearch);
        type_text(&mut v, "合計");
        assert_eq!(v.cursor(), (0, 0), "the merge under the cursor comes first");
        let state = v.search_state().unwrap();
        assert_eq!((state.current, state.total), (1, 2));
        v.apply(Event::CancelEdit);
        assert_eq!(v.cursor(), (0, 1), "Esc restores the covered cell, raw");
    }

    #[test]
    fn first_match_starts_at_the_origin_and_wraps() {
        let matches = [(0, 2), (2, 2)];
        assert_eq!(first_at_or_after(&matches, (0, 0)), 0);
        assert_eq!(first_at_or_after(&matches, (1, 0)), 1);
        assert_eq!(
            first_at_or_after(&matches, (2, 3)),
            0,
            "past the last: wrap"
        );
    }

    #[test]
    fn typing_jumps_to_the_nearest_match_below_the_origin() {
        let mut v = search_viewer(search_sheet());
        v.apply(Event::Move { rows: 1, cols: 0 });
        v.apply(Event::OpenSearch);
        type_text(&mut v, "合計");
        assert_eq!(v.cursor(), (2, 2), "the match after row 1, not (0,2)");
        let state = v.search_state().unwrap();
        assert_eq!((state.current, state.total), (2, 2));
    }

    #[test]
    fn arrows_cycle_matches_and_wrap() {
        let mut v = search_viewer(search_sheet());
        v.apply(Event::OpenSearch);
        type_text(&mut v, "合計");
        assert_eq!(v.cursor(), (0, 2));
        v.apply(Event::Move { rows: 1, cols: 0 });
        assert_eq!(v.cursor(), (2, 2));
        v.apply(Event::Move { rows: 1, cols: 0 });
        assert_eq!(v.cursor(), (0, 2), "wraps forward");
        v.apply(Event::Move { rows: -1, cols: 0 });
        assert_eq!(v.cursor(), (2, 2), "wraps backward");
    }

    #[test]
    fn enter_stays_and_esc_returns_to_the_origin() {
        let mut v = search_viewer(search_sheet());
        v.apply(Event::OpenSearch);
        type_text(&mut v, "みかん");
        assert_eq!(v.cursor(), (2, 0));
        v.apply(Event::Submit);
        assert_eq!(*v.mode(), Mode::Grid);
        assert_eq!(v.cursor(), (2, 0), "Enter keeps the found cell");

        v.apply(Event::OpenSearch);
        type_text(&mut v, "りんご");
        assert_eq!(v.cursor(), (1, 0));
        v.apply(Event::CancelEdit);
        assert_eq!(*v.mode(), Mode::Grid);
        assert_eq!(v.cursor(), (2, 0), "Esc goes back to where search began");
    }

    #[test]
    fn no_match_keeps_the_cursor_at_the_origin_and_reports_zero() {
        let mut v = search_viewer(search_sheet());
        v.apply(Event::Move { rows: 1, cols: 1 });
        v.apply(Event::OpenSearch);
        type_text(&mut v, "zzz");
        assert_eq!(v.cursor(), (1, 1));
        let state = v.search_state().unwrap();
        assert_eq!((state.current, state.total), (0, 0));
        v.apply(Event::Move { rows: 1, cols: 0 });
        assert_eq!(v.cursor(), (1, 1), "cycling with no matches is a no-op");
    }

    #[test]
    fn shrinking_the_query_rescans_from_the_origin() {
        let mut v = search_viewer(search_sheet());
        v.apply(Event::OpenSearch);
        type_text(&mut v, "合計欄");
        assert_eq!(v.cursor(), (2, 2));
        v.apply(Event::Backspace);
        assert_eq!(v.cursor(), (0, 2), "'合計' finds the earlier match again");
        let state = v.search_state().unwrap();
        assert_eq!((state.current, state.total), (1, 2));
    }

    #[test]
    fn a_document_reload_recomputes_the_matches_mid_search() {
        let source = SharedSource::new(vec![search_sheet()]);
        let mut v = viewer_on(&source);
        v.apply(Event::OpenSearch);
        type_text(&mut v, "合計");
        let state = v.search_state().unwrap();
        assert_eq!((state.current, state.total), (1, 2));
        source.write_from_outside(vec![Sheet::new(
            "s",
            vec![vec![text("x"), text("合計だけ")]],
        )]);
        v.apply(Event::Tick);
        let state = v.search_state().unwrap();
        assert_eq!((state.current, state.total), (1, 1));
        v.apply(Event::Move { rows: 1, cols: 0 });
        assert_eq!(v.cursor(), (0, 1), "cycling lands on the new match");
    }

    #[test]
    fn search_state_is_none_outside_search_mode() {
        let v = viewer_with(3, 3, Vec::new(), Box::new(NullStore));
        assert!(v.search_state().is_none());
    }
}
