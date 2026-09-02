use crate::domain::sheet::Sheet;

use super::{Event, Notice, Viewer};

/// Common terminals silently drop OSC 52 payloads past about 100KB.
const COPY_LIMIT_BYTES: usize = 100_000;

impl Viewer {
    pub(super) fn apply_mouse(&mut self, event: Event) {
        let max_row = self.sheet().row_count().saturating_sub(1);
        let max_col = self.sheet().col_count().saturating_sub(1);
        let clamp = |row: usize, col: usize| (row.min(max_row), col.min(max_col));
        match event {
            Event::SelectCell { row, col } => {
                let cell = clamp(row, col);
                self.selection = Some((cell, cell));
                self.set_cursor(cell);
            }
            Event::SelectSheet(index) => {
                self.selection = None;
                if index < self.sheets.len() {
                    self.active = index;
                }
            }
            Event::DragTo { row, col } => {
                if let Some((_, current)) = &mut self.selection {
                    *current = clamp(row, col);
                }
            }
            Event::DragEnd { copy } => {
                if let (Some((start, end)), true) = (self.selection.take(), copy) {
                    let rows = start.0.abs_diff(end.0) + 1;
                    let cols = start.1.abs_diff(end.1) + 1;
                    let text = tsv(self.sheet(), start, end);
                    if text.len() > COPY_LIMIT_BYTES {
                        self.notice = Some(Notice::Copy("Selection too large to copy".to_string()));
                    } else {
                        self.copy_request = Some(text);
                        self.notice = Some(Notice::Copy(format!("Copied {rows}×{cols} cells")));
                    }
                }
            }
            _ => {}
        }
    }
}

/// Full displayed texts; tabs and line breaks inside a cell become spaces.
fn tsv(sheet: &Sheet, a: (usize, usize), b: (usize, usize)) -> String {
    let (r0, r1) = (a.0.min(b.0), a.0.max(b.0));
    let (c0, c1) = (a.1.min(b.1), a.1.max(b.1));
    (r0..=r1)
        .map(|r| {
            (c0..=c1)
                .map(|c| {
                    sheet
                        .cell(r, c)
                        .display_text()
                        .replace(['\t', '\n', '\r'], " ")
                })
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use crate::domain::cell::CellValue;
    use crate::domain::document::Document;
    use crate::domain::sheet::Sheet;

    use super::super::test_support::{NullStore, type_text, viewer_with};
    use super::super::{Event, Mode, Viewer};

    fn text(s: &str) -> CellValue {
        CellValue::Text(s.into())
    }

    fn grid_viewer() -> Viewer {
        let sheet = Sheet::new(
            "s",
            vec![
                vec![text("項目"), text("単価"), text("数量")],
                vec![
                    text("りんご"),
                    CellValue::Number(120.0),
                    CellValue::Number(3.0),
                ],
                vec![
                    text("み\nかん"),
                    CellValue::Number(80.0),
                    CellValue::Number(5.0),
                ],
            ],
        );
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
    fn a_click_moves_the_cursor_and_clamps_to_the_sheet() {
        let mut v = grid_viewer();
        v.apply(Event::SelectCell { row: 1, col: 2 });
        assert_eq!(v.cursor(), (1, 2));
        v.apply(Event::SelectCell { row: 99, col: 99 });
        assert_eq!(v.cursor(), (2, 2), "clicks beyond the sheet clamp");
    }

    #[test]
    fn a_drag_selects_and_release_copies_tsv() {
        let mut v = grid_viewer();
        v.apply(Event::SelectCell { row: 0, col: 0 });
        v.apply(Event::DragTo { row: 1, col: 1 });
        assert_eq!(v.selection(), Some(((0, 0), (1, 1))));
        v.apply(Event::DragEnd { copy: true });
        assert_eq!(v.selection(), None, "selection is transient");
        assert_eq!(
            v.take_copy_request().unwrap(),
            "項目\t単価\nりんご\t120",
            "full values, tabs between cells, newlines between rows"
        );
        assert_eq!(v.notice(), Some("Copied 2×2 cells"));
        v.apply(Event::Move { rows: 1, cols: 0 });
        assert_eq!(v.notice(), None, "the receipt retires on the next input");
    }

    #[test]
    fn a_backwards_drag_normalizes_and_cell_breaks_flatten() {
        let mut v = grid_viewer();
        v.apply(Event::SelectCell { row: 2, col: 1 });
        v.apply(Event::DragTo { row: 2, col: 0 });
        v.apply(Event::DragEnd { copy: true });
        assert_eq!(
            v.take_copy_request().unwrap(),
            "み かん\t80",
            "dragging right-to-left still reads left-to-right; \\n becomes a space"
        );
    }

    #[test]
    fn clicks_win_over_open_prompts_in_one_motion() {
        let mut v = viewer_with(3, 3, Vec::new(), Box::new(NullStore));
        v.apply(Event::StartComment);
        type_text(&mut v, "half-typed");
        v.apply(Event::SelectCell { row: 2, col: 1 });
        assert_eq!(*v.mode(), Mode::Grid);
        assert_eq!(v.cursor(), (2, 1));

        v.apply(Event::OpenSheetPicker);
        v.apply(Event::SelectCell { row: 0, col: 0 });
        assert_eq!(*v.mode(), Mode::Grid);
        assert_eq!(v.cursor(), (0, 0));

        v.apply(Event::Move { rows: 1, cols: 1 });
        v.apply(Event::OpenSearch);
        v.apply(Event::SelectCell { row: 2, col: 2 });
        assert_eq!(*v.mode(), Mode::Grid);
        assert_eq!(v.cursor(), (2, 2));
    }

    #[test]
    fn a_plain_click_release_dissolves_the_selection_without_copying() {
        let mut v = grid_viewer();
        v.apply(Event::SelectCell { row: 1, col: 1 });
        v.apply(Event::DragEnd { copy: false });
        assert_eq!(v.selection(), None, "no second cursor left behind");
        assert!(v.take_copy_request().is_none());
        assert_eq!(v.notice(), None);
    }

    #[test]
    fn any_key_event_dissolves_a_stale_selection() {
        let mut v = viewer_with(3, 3, Vec::new(), Box::new(NullStore));
        // a lost Up event leaves the press armed
        v.apply(Event::SelectCell { row: 1, col: 1 });
        v.apply(Event::NextSheet);
        assert_eq!(v.selection(), None, "sheet switches clear it");
        v.apply(Event::DragTo { row: 0, col: 0 });
        v.apply(Event::DragEnd { copy: true });
        assert!(
            v.take_copy_request().is_none(),
            "a drag that died cannot copy the other sheet"
        );

        v.apply(Event::SelectCell { row: 1, col: 1 });
        v.apply(Event::Move { rows: 1, cols: 0 });
        assert_eq!(v.selection(), None, "keyboard movement clears it");
    }

    #[test]
    fn an_oversized_selection_refuses_to_copy() {
        let big = "x".repeat(60_000);
        let sheet = Sheet::new(
            "s",
            vec![vec![CellValue::Text(big.clone()), CellValue::Text(big)]],
        );
        let mut v = Viewer::from_document(
            Document::new(vec![sheet]),
            Vec::new(),
            None,
            None,
            Box::new(NullStore),
        )
        .unwrap();
        v.apply(Event::SelectCell { row: 0, col: 0 });
        v.apply(Event::DragTo { row: 0, col: 1 });
        v.apply(Event::DragEnd { copy: true });
        assert!(v.take_copy_request().is_none(), "nothing sent");
        assert_eq!(v.notice(), Some("Selection too large to copy"));
    }

    #[test]
    fn the_copy_receipt_survives_pointer_motion() {
        let mut v = grid_viewer();
        v.apply(Event::SelectCell { row: 0, col: 0 });
        v.apply(Event::DragTo { row: 0, col: 1 });
        v.apply(Event::DragEnd { copy: true });
        v.take_copy_request();
        assert_eq!(v.notice(), Some("Copied 1×2 cells"));
        v.apply(Event::Noop);
        assert_eq!(
            v.notice(),
            Some("Copied 1×2 cells"),
            "bare pointer motion must not eat the receipt"
        );
        v.apply(Event::Move { rows: 1, cols: 0 });
        assert_eq!(v.notice(), None, "a real input still retires it");
    }

    #[test]
    fn a_tab_click_switches_sheets_and_ignores_bad_indices() {
        let mut v = viewer_with(3, 3, Vec::new(), Box::new(NullStore));
        v.apply(Event::SelectSheet(1));
        assert_eq!(v.active(), 1);
        v.apply(Event::SelectSheet(9));
        assert_eq!(v.active(), 1, "an out-of-range tab does nothing");
    }
}
