//! The sheet-picker mode: filtering sheet names and switching on Enter.

use crate::domain::anchor::Anchor;

use super::matching::{contains_folded, fold};
use super::{Event, Mode, Viewer, add_clamped};

/// What the picker shows this frame; `candidates` are sheet indices in
/// workbook order.
pub struct PickerState<'a> {
    pub query: &'a str,
    pub selected: usize,
    pub candidates: Vec<usize>,
}

impl Viewer {
    pub fn picker_state(&self) -> Option<PickerState<'_>> {
        let Mode::SheetPicker { query, selected } = &self.mode else {
            return None;
        };
        let needle = fold(query);
        let candidates = self
            .sheet_names()
            .iter()
            .enumerate()
            .filter(|(_, name)| contains_folded(name, &needle))
            .map(|(i, _)| i)
            .collect();
        Some(PickerState {
            query,
            selected: *selected,
            candidates,
        })
    }

    /// Unresolved threads per sheet, aligned with `sheet_names()`. Indexed
    /// through a map: the sidecar is agent-written and can hold thousands of
    /// threads, and this runs on every keystroke while the picker is open.
    pub fn unresolved_counts(&self) -> Vec<usize> {
        let names = self.sheet_names();
        let index: std::collections::HashMap<&str, usize> = names
            .iter()
            .enumerate()
            .map(|(i, name)| (*name, i))
            .collect();
        let mut counts = vec![0; names.len()];
        for thread in self.comments.iter().filter(|t| !t.resolved) {
            let Anchor::Cell { sheet, .. } = &thread.anchor;
            if let Some(&i) = index.get(sheet.as_str()) {
                counts[i] += 1;
            }
        }
        counts
    }

    /// After a document reload the sheet list may have shrunk under the
    /// picker; the selection is pulled back onto the last candidate.
    pub(super) fn clamp_picker_selection(&mut self) {
        let count = match self.picker_state() {
            Some(state) => state.candidates.len(),
            None => return,
        };
        if let Mode::SheetPicker { selected, .. } = &mut self.mode {
            *selected = (*selected).min(count.saturating_sub(1));
        }
    }

    pub(super) fn apply_picker(&mut self, event: Event) {
        let candidates = match self.picker_state() {
            Some(state) => state.candidates,
            None => return,
        };
        let Mode::SheetPicker { query, selected } = &mut self.mode else {
            return;
        };
        match event {
            // every edit resets the selection: "type, then arrow down" always
            // behaves the same, whatever survived the previous filter
            Event::Insert(c) => {
                query.push(c);
                *selected = 0;
            }
            // only an actual edit resets — Backspace on an empty query must
            // not throw away the selection the picker opened on
            Event::Backspace => {
                if query.pop().is_some() {
                    *selected = 0;
                }
            }
            Event::Move { rows, .. } => {
                if !candidates.is_empty() {
                    let max = candidates.len() - 1;
                    *selected = add_clamped(*selected, rows, max);
                }
            }
            // Enter only — a single remaining candidate never auto-switches
            Event::Submit => {
                if let Some(&sheet) = candidates.get(*selected) {
                    self.active = sheet;
                    self.mode = Mode::Grid;
                }
            }
            Event::CancelEdit => self.mode = Mode::Grid,
            Event::Quit => self.quit = true,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::cell::CellValue;
    use crate::domain::document::Document;
    use crate::domain::sheet::Sheet;

    use super::super::test_support::{NullStore, SharedStore, thread, type_text, viewer_with};
    use super::*;

    fn viewer_named(names: &[&str]) -> Viewer {
        let sheets = names
            .iter()
            .map(|n| Sheet::new(*n, vec![vec![CellValue::Number(1.0)]]))
            .collect();
        Viewer::from_document(
            Document::new(sheets),
            Vec::new(),
            None,
            None,
            Box::new(NullStore),
        )
        .unwrap()
    }

    #[test]
    fn the_picker_opens_on_the_active_sheet_and_esc_changes_nothing() {
        let mut v = viewer_named(&["概要", "IT-01", "IT-02"]);
        v.apply(Event::NextSheet);
        v.apply(Event::OpenSheetPicker);
        let state = v.picker_state().unwrap();
        assert_eq!(state.query, "");
        assert_eq!(state.selected, 1, "the active sheet starts selected");
        assert_eq!(state.candidates, vec![0, 1, 2], "empty query lists all");
        v.apply(Event::CancelEdit);
        assert_eq!(*v.mode(), Mode::Grid);
        assert_eq!(v.active(), 1, "cancelling switches nothing");
    }

    #[test]
    fn the_filter_folds_case_and_full_width_characters() {
        let mut v = viewer_named(&["概要", "IT-01", "IT-02", "不具合等報告"]);
        v.apply(Event::OpenSheetPicker);
        // an IME left on full-width: ｉｔ－0 must still find IT-01 and IT-02
        type_text(&mut v, "ｉｔ－0");
        assert_eq!(v.picker_state().unwrap().candidates, vec![1, 2]);

        v.apply(Event::CancelEdit);
        v.apply(Event::OpenSheetPicker);
        type_text(&mut v, "不具合");
        assert_eq!(v.picker_state().unwrap().candidates, vec![3]);
    }

    #[test]
    fn an_edit_resets_the_selection_to_the_first_match() {
        let mut v = viewer_named(&["one", "two", "three"]);
        v.apply(Event::OpenSheetPicker);
        v.apply(Event::Move { rows: 1, cols: 0 });
        assert_eq!(v.picker_state().unwrap().selected, 1);
        v.apply(Event::Insert('t'));
        assert_eq!(v.picker_state().unwrap().selected, 0, "typing resets");
        v.apply(Event::Move { rows: 1, cols: 0 });
        v.apply(Event::Backspace);
        assert_eq!(v.picker_state().unwrap().selected, 0, "deleting resets");
    }

    #[test]
    fn backspace_on_an_empty_query_keeps_the_selection() {
        let mut v = viewer_named(&["one", "two", "three"]);
        v.apply(Event::NextSheet);
        v.apply(Event::NextSheet);
        v.apply(Event::OpenSheetPicker);
        assert_eq!(v.picker_state().unwrap().selected, 2);
        v.apply(Event::Backspace);
        assert_eq!(
            v.picker_state().unwrap().selected,
            2,
            "nothing was edited, so nothing resets"
        );
    }

    #[test]
    fn the_selection_clamps_to_the_candidate_list() {
        let mut v = viewer_named(&["one", "two", "three"]);
        v.apply(Event::OpenSheetPicker);
        v.apply(Event::Move { rows: -1, cols: 0 });
        assert_eq!(v.picker_state().unwrap().selected, 0);
        v.apply(Event::Move { rows: 10, cols: 0 });
        assert_eq!(v.picker_state().unwrap().selected, 2);
    }

    #[test]
    fn enter_switches_and_a_single_match_never_switches_on_its_own() {
        let mut v = viewer_named(&["one", "two", "three"]);
        v.apply(Event::OpenSheetPicker);
        type_text(&mut v, "thr");
        assert_eq!(v.picker_state().unwrap().candidates, vec![2]);
        assert_eq!(v.active(), 0, "one candidate left, still not switched");
        v.apply(Event::Submit);
        assert_eq!(*v.mode(), Mode::Grid);
        assert_eq!(v.active(), 2);
    }

    #[test]
    fn enter_with_no_match_keeps_the_picker_open() {
        let mut v = viewer_named(&["one", "two"]);
        v.apply(Event::OpenSheetPicker);
        type_text(&mut v, "zzz");
        v.apply(Event::Submit);
        assert!(matches!(v.mode(), Mode::SheetPicker { .. }));
        assert_eq!(v.active(), 0);
    }

    #[test]
    fn unresolved_counts_align_with_sheet_names() {
        let comments = vec![
            thread("one", 0, 0, false),
            thread("one", 1, 1, false),
            thread("two", 0, 0, true),
        ];
        let v = viewer_with(3, 3, comments, Box::new(NullStore));
        assert_eq!(v.unresolved_counts(), vec![2, 0]);
    }

    #[test]
    fn a_tick_reloads_while_the_picker_is_open_without_touching_it() {
        let store = SharedStore::default();
        let shared = store.clone();
        let mut v = viewer_with(3, 3, Vec::new(), Box::new(store));
        v.apply(Event::OpenSheetPicker);
        type_text(&mut v, "tw");

        shared.write_from_outside(vec![thread("two", 0, 0, false)]);
        v.apply(Event::Tick);

        assert_eq!(v.unresolved_counts(), vec![0, 1], "counts refresh");
        let state = v.picker_state().unwrap();
        assert_eq!(state.query, "tw", "typing survives the reload");
        assert_eq!(state.candidates, vec![1]);
    }
}
