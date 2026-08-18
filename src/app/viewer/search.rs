#[cfg(test)]
mod tests {
    use crate::domain::cell::CellValue;
    use crate::domain::document::Document;
    use crate::domain::sheet::{MergedRange, Sheet};

    use super::super::test_support::{NullStore, type_text, viewer_with};
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
                color: None,
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
    fn search_state_is_none_outside_search_mode() {
        let v = viewer_with(3, 3, Vec::new(), Box::new(NullStore));
        assert!(v.search_state().is_none());
    }
}
