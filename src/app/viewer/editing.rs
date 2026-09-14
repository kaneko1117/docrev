use crate::app::comments;
use crate::domain::anchor::Anchor;
use crate::domain::comment::CommentThread;
use crate::domain::sheet::MergedRange;

use super::{Body, EditTarget, Event, Mode, Notice, Viewer};

impl Viewer {
    pub(super) fn apply_editing(&mut self, event: Event) {
        if Self::moves_cursor(event) {
            // an emptied text document leaves the cursor nowhere to go
            if self.text().is_some_and(|text| text.is_empty()) {
                return;
            }
            self.close_editor();
            match self.body {
                Body::Grid(_) => self.apply_grid(event),
                Body::Text(_) => self.apply_text(event),
            }
            self.start_comment();
            return;
        }
        let Mode::Editing { buffer, .. } = &mut self.mode else {
            return;
        };
        match event {
            Event::Insert(c) => buffer.push(c),
            Event::Newline => buffer.push('\n'),
            Event::Backspace => {
                buffer.pop();
            }
            Event::CancelEdit => self.close_editor(),
            Event::Submit => self.submit(),
            _ => {}
        }
    }

    fn moves_cursor(event: Event) -> bool {
        Self::is_mouse(event)
            || matches!(
                event,
                Event::Move { .. }
                    | Event::Top
                    | Event::Bottom
                    | Event::RowStart
                    | Event::RowEnd
                    | Event::NextSheet
                    | Event::PrevSheet
            )
    }

    /// Opens the editor on the cursor's place with that place's draft; does nothing on an empty text document.
    pub(super) fn start_comment(&mut self) {
        let at = match &self.body {
            Body::Grid(grid) => {
                let (row, col) = grid.cursor();
                let sheet = grid.sheet();
                // a merged region is one place, so it holds one draft
                let (row, col) = sheet
                    .merge_at(row, col)
                    .map_or((row, col), MergedRange::anchor);
                Anchor::cell(sheet.name(), row as u32, col as u32)
            }
            Body::Text(text) if text.document().is_empty() => return,
            Body::Text(text) => Anchor::line(text.line() as u32),
        };
        let target = match self.thread_at_cursor() {
            Some(_) => EditTarget::Reply,
            None => EditTarget::NewThread,
        };
        let buffer = self.take_drafts(&at);
        self.mode = Mode::Editing { target, at, buffer };
    }

    /// The place's own draft, after the text of every draft whose place a reload cut away.
    fn take_drafts(&mut self, at: &Anchor) -> String {
        let mut places: Vec<Anchor> = self
            .drafts
            .keys()
            .filter(|place| !self.still_exists(place))
            .cloned()
            .collect();
        places.sort_by_key(Anchor::label);
        places.push(at.clone());
        places
            .iter()
            .filter_map(|place| self.drafts.remove(place))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// False once the sheet, row, column or line is gone; a hidden or merged cell still exists.
    fn still_exists(&self, at: &Anchor) -> bool {
        match (at, &self.body) {
            (Anchor::Cell { sheet, row, col }, Body::Grid(grid)) => {
                // the cursor sits at (0, 0) even on an empty sheet
                grid.sheet_named(sheet).is_some_and(|sheet| {
                    (*row as usize) < sheet.row_count().max(1)
                        && (*col as usize) < sheet.col_count().max(1)
                })
            }
            (Anchor::Line { line }, Body::Text(text)) => (*line as usize) < text.document().len(),
            _ => false,
        }
    }

    /// Blank text leaves no draft behind.
    fn close_editor(&mut self) {
        match std::mem::replace(&mut self.mode, Mode::Grid) {
            Mode::Editing { at, buffer, .. } if !buffer.trim().is_empty() => {
                self.drafts.insert(at, buffer);
            }
            _ => {}
        }
    }

    /// Empty input closes without saving; a failed save keeps the editor open. The thread is
    /// chosen at save time, so one that appeared on the place while typing is continued, not forked.
    fn submit(&mut self) {
        let Mode::Editing { at, buffer, .. } = &self.mode else {
            return;
        };
        let body = buffer.trim().to_string();
        if body.is_empty() {
            self.mode = Mode::Grid;
            return;
        }
        let saved = match at.clone() {
            Anchor::Cell { sheet, row, col } => {
                self.save_on_cell(&sheet, row as usize, col as usize, &body)
            }
            Anchor::Line { line } => self.save_on_line(line as usize, &body),
        };
        match saved {
            Ok(thread) => {
                match self.comments.iter_mut().find(|t| t.id == thread.id) {
                    Some(existing) => *existing = thread,
                    None => self.comments.push(thread),
                }
                self.mode = Mode::Grid;
                self.notice = None;
                // `revision` is deliberately not refreshed: the next tick must
                // reload, or a write that landed while typing is lost
            }
            Err(reason) => self.notice = Some(Notice::Save(format!("save failed: {reason}"))),
        }
    }

    /// `Err` carries what to tell the user; the editor stays open.
    fn save_on_cell(
        &mut self,
        name: &str,
        row: usize,
        col: usize,
        body: &str,
    ) -> Result<CommentThread, String> {
        // field borrows: the sheet and the store must be held at once
        let (Body::Grid(grid), store) = (&self.body, self.store.as_mut()) else {
            return Err("not a cell".to_string());
        };
        let sheet = grid
            .sheet_named(name)
            .ok_or_else(|| format!("sheet {name:?} is gone"))?;
        comments::comment_on_cell(store, sheet, row, col, body, "user").map_err(|e| e.to_string())
    }

    /// The line may have been cut away by a reload while the editor was open.
    fn save_on_line(&mut self, line: usize, body: &str) -> Result<CommentThread, String> {
        let (Body::Text(text), store) = (&self.body, self.store.as_mut()) else {
            return Err("not a line".to_string());
        };
        if line >= text.document().len() {
            return Err(format!("line {} is gone", line + 1));
        }
        comments::comment_on_line(store, line as u32, body, "user").map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::app::error::StoreError;
    use crate::app::ports::CommentStore;
    use crate::domain::cell::CellValue;
    use crate::domain::comment::Reply;
    use crate::domain::document::Document;
    use crate::domain::sheet::Sheet;

    use super::super::EditTarget;
    use super::super::test_support::{
        LiveStore, NullStore, RecordingStore, SharedSource, text_source, text_viewer_with_store,
        thread, type_text, viewer, viewer_on_with, viewer_with,
    };
    use super::*;

    #[test]
    fn typing_builds_the_buffer() {
        let mut v = viewer(3, 3);
        v.apply(Event::StartComment);
        type_text(&mut v, "line1\nline2");
        v.apply(Event::Backspace);
        match v.mode() {
            Mode::Editing { buffer, .. } => assert_eq!(buffer, "line1\nline"),
            other => panic!("expected editing mode, got {other:?}"),
        }
    }

    /// (anchor label, buffer) of the open editor.
    fn editor(v: &Viewer) -> (String, &str) {
        match v.mode() {
            Mode::Editing { at, buffer, .. } => (at.label(), buffer),
            other => panic!("expected editing mode, got {other:?}"),
        }
    }

    #[test]
    fn escape_closes_without_saving_and_keeps_the_draft() {
        let store = RecordingStore::default();
        let log = store.log.clone();
        let mut v = viewer_with(3, 3, Vec::new(), Box::new(store));
        v.apply(Event::StartComment);
        type_text(&mut v, "draft");
        v.apply(Event::CancelEdit);
        assert_eq!(*v.mode(), Mode::Grid);
        assert!(log.borrow().is_empty());
        v.apply(Event::StartComment);
        assert_eq!(editor(&v), ("one!A1".to_string(), "draft"));
    }

    #[test]
    fn the_editor_follows_the_cursor_and_each_cell_keeps_its_draft() {
        let store = RecordingStore::default();
        let log = store.log.clone();
        let mut v = viewer_with(4, 3, Vec::new(), Box::new(store));
        v.apply(Event::Move { rows: 2, cols: 1 });
        v.apply(Event::StartComment);
        type_text(&mut v, "単価が古い?");
        v.apply(Event::Move { rows: 1, cols: 0 });
        assert_eq!(v.cursor(), (3, 1));
        assert_eq!(editor(&v), ("one!B4".to_string(), ""));
        type_text(&mut v, "合計ずれ");
        v.apply(Event::Move { rows: -1, cols: 0 });
        assert_eq!(editor(&v), ("one!B3".to_string(), "単価が古い?"));
        v.apply(Event::Submit);
        assert_eq!(*v.mode(), Mode::Grid);
        assert_eq!(log.borrow().as_slice(), ["thread B3 単価が古い?"]);
        v.apply(Event::Move { rows: 1, cols: 0 });
        v.apply(Event::StartComment);
        assert_eq!(editor(&v), ("one!B4".to_string(), "合計ずれ"));
        v.apply(Event::Move { rows: -1, cols: 0 });
        assert_eq!(
            editor(&v),
            ("one!B3".to_string(), ""),
            "a saved draft is gone"
        );
    }

    #[test]
    fn moving_onto_a_thread_turns_the_editor_into_a_reply_and_blank_text_is_no_draft() {
        let comments = vec![thread("one", 1, 0, false)];
        let mut v = viewer_with(3, 3, comments, Box::new(NullStore));
        v.apply(Event::StartComment);
        type_text(&mut v, " \n");
        v.apply(Event::Move { rows: 1, cols: 0 });
        assert!(matches!(
            v.mode(),
            Mode::Editing {
                target: EditTarget::Reply,
                ..
            }
        ));
        v.apply(Event::Move { rows: -1, cols: 0 });
        assert!(matches!(
            v.mode(),
            Mode::Editing { target: EditTarget::NewThread, buffer, .. } if buffer.is_empty()
        ));
    }

    #[test]
    fn switching_sheets_while_editing_keeps_a_draft_per_sheet() {
        let mut v = viewer(3, 3);
        v.apply(Event::StartComment);
        type_text(&mut v, "on one");
        v.apply(Event::NextSheet);
        assert_eq!(editor(&v), ("two!A1".to_string(), ""));
        v.apply(Event::PrevSheet);
        assert_eq!(editor(&v), ("one!A1".to_string(), "on one"));
    }

    #[test]
    fn submit_saves_a_thread_on_the_cursor_cell() {
        let store = RecordingStore::default();
        let log = store.log.clone();
        let mut v = viewer_with(3, 3, Vec::new(), Box::new(store));
        v.apply(Event::Move { rows: 1, cols: 1 });
        v.apply(Event::StartComment);
        type_text(&mut v, "check this");
        v.apply(Event::Submit);
        assert_eq!(*v.mode(), Mode::Grid);
        assert_eq!(log.borrow().as_slice(), ["thread B2 check this"]);
        assert_eq!(v.unresolved_on_active_sheet(), vec![(1, 1)]);
    }

    #[test]
    fn empty_submit_closes_without_saving() {
        let store = RecordingStore::default();
        let log = store.log.clone();
        let mut v = viewer_with(3, 3, Vec::new(), Box::new(store));
        v.apply(Event::StartComment);
        type_text(&mut v, "  \n ");
        v.apply(Event::Submit);
        assert_eq!(*v.mode(), Mode::Grid);
        assert!(log.borrow().is_empty());
    }

    #[test]
    fn c_replies_to_the_thread_under_the_cursor() {
        let comments = vec![thread("one", 0, 0, false)];
        let store = RecordingStore::seeded(comments.clone());
        let log = store.log.clone();
        let mut v = viewer_with(3, 3, comments, Box::new(store));
        v.apply(Event::StartComment);
        type_text(&mut v, "done");
        v.apply(Event::Submit);
        assert_eq!(log.borrow().as_slice(), ["reply t-one-0-0 done"]);
        let updated = v.thread_at_cursor().unwrap();
        assert_eq!(updated.replies.len(), 1);
    }

    #[test]
    fn c_on_an_open_thread_replies_instead_of_forking() {
        let comments = vec![thread("one", 0, 0, false)];
        let mut v = viewer_with(3, 3, comments, Box::new(NullStore));
        v.apply(Event::StartComment);
        assert!(matches!(
            v.mode(),
            Mode::Editing {
                target: EditTarget::Reply,
                ..
            }
        ));
    }

    #[test]
    fn c_on_a_resolved_thread_continues_it() {
        let comments = vec![thread("one", 0, 0, true)];
        let store = RecordingStore::seeded(comments.clone());
        let log = store.log.clone();
        let mut v = viewer_with(3, 3, comments, Box::new(store));
        v.apply(Event::StartComment);
        assert!(matches!(
            v.mode(),
            Mode::Editing {
                target: EditTarget::Reply,
                ..
            }
        ));
        type_text(&mut v, "one more thing");
        v.apply(Event::Submit);
        assert_eq!(log.borrow().as_slice(), ["reply t-one-0-0 one more thing"]);
        assert!(
            v.thread_at_cursor().is_some_and(|t| !t.resolved),
            "the reply reopens the conversation"
        );
        assert_eq!(v.unresolved_on_active_sheet(), vec![(0, 0)]);
    }

    #[test]
    fn c_on_a_cell_without_a_thread_starts_one() {
        let mut v = viewer(3, 3);
        v.apply(Event::StartComment);
        assert!(matches!(
            v.mode(),
            Mode::Editing {
                target: EditTarget::NewThread,
                ..
            }
        ));
    }

    #[test]
    fn a_cell_with_legacy_threads_continues_the_open_one_else_the_newest() {
        let mut older = thread("one", 0, 0, true);
        older.id = "older".into();
        let mut newer = thread("one", 0, 0, true);
        newer.id = "newer".into();
        let both = vec![older.clone(), newer.clone()];
        let store = RecordingStore::seeded(both.clone());
        let log = store.log.clone();
        let mut v = viewer_with(3, 3, both, Box::new(store));
        assert_eq!(v.thread_at_cursor().map(|t| t.id.as_str()), Some("newer"));
        v.apply(Event::StartComment);
        type_text(&mut v, "again");
        v.apply(Event::Submit);
        assert_eq!(log.borrow().as_slice(), ["reply newer again"]);

        older.resolved = false;
        let both = vec![older, newer];
        let store = RecordingStore::seeded(both.clone());
        let log = store.log.clone();
        let mut v = viewer_with(3, 3, both, Box::new(store));
        assert_eq!(v.thread_at_cursor().map(|t| t.id.as_str()), Some("older"));
        v.apply(Event::StartComment);
        type_text(&mut v, "again");
        v.apply(Event::Submit);
        assert_eq!(log.borrow().as_slice(), ["reply older again"]);
    }

    #[test]
    fn a_reply_follows_its_cell_when_a_reload_moves_the_cursor() {
        let rows = |n| vec![vec![CellValue::Number(1.0); 3]; n];
        let source = SharedSource::new(vec![Sheet::new("one", rows(3))]);
        let store = LiveStore::default();
        let shared = store.clone();
        shared.threads.borrow_mut().push(thread("one", 2, 2, false));
        let mut v = viewer_on_with(&source, Box::new(store));
        v.apply(Event::Move { rows: 2, cols: 2 });
        v.apply(Event::StartComment);
        assert!(matches!(
            v.mode(),
            Mode::Editing {
                target: EditTarget::Reply,
                ..
            }
        ));
        type_text(&mut v, "fix this");

        // the agent drops the last row; the reload pulls the cursor up to C2
        source.write_from_outside(vec![Sheet::new("one", rows(2))]);
        v.apply(Event::Tick);
        assert_eq!(v.cursor(), (1, 2));

        v.apply(Event::Submit);
        assert_eq!(*v.mode(), Mode::Grid);
        let threads = shared.threads.borrow();
        assert_eq!(
            threads.len(),
            1,
            "nothing lands on the cell the cursor drifted to"
        );
        assert_eq!(threads[0].replies[0].body, "fix this");
    }

    #[test]
    fn a_reply_whose_sheet_vanished_keeps_the_draft_and_reports_it() {
        let one_cell = || vec![vec![CellValue::Number(1.0)]];
        let source = SharedSource::new(vec![Sheet::new("one", one_cell())]);
        let store = LiveStore::default();
        let shared = store.clone();
        shared.threads.borrow_mut().push(thread("one", 0, 0, false));
        let mut v = viewer_on_with(&source, Box::new(store));
        v.apply(Event::StartComment);
        type_text(&mut v, "fix this");

        source.write_from_outside(vec![Sheet::new("renamed", one_cell())]);
        v.apply(Event::Tick);

        v.apply(Event::Submit);
        assert!(
            matches!(v.mode(), Mode::Editing { .. }),
            "the draft is kept"
        );
        assert!(v.notice().is_some_and(|n| n.contains("save failed")));
        let threads = shared.threads.borrow();
        assert_eq!(threads.len(), 1);
        assert!(threads[0].replies.is_empty());
    }

    #[test]
    fn a_thread_that_appears_while_typing_is_continued_not_forked() {
        let store = LiveStore::default();
        let shared = store.clone();
        let mut v = viewer_with(3, 3, Vec::new(), Box::new(store));
        v.apply(Event::Move { rows: 2, cols: 2 });
        v.apply(Event::StartComment);
        assert!(matches!(
            v.mode(),
            Mode::Editing {
                target: EditTarget::NewThread,
                ..
            }
        ));
        type_text(&mut v, "mine");

        shared.write_from_outside(vec![thread("one", 2, 2, false)]);

        v.apply(Event::Submit);
        v.apply(Event::Tick);

        let threads = shared.threads.borrow();
        assert_eq!(threads.len(), 1, "no second thread on the cell");
        assert_eq!(threads[0].replies.len(), 1);
        assert_eq!(threads[0].replies[0].body, "mine");
        assert_eq!(v.thread_at_cursor().map(|t| t.replies.len()), Some(1));
    }

    #[test]
    fn a_merged_region_holds_one_draft() {
        use crate::domain::sheet::MergedRange;
        let sheet =
            Sheet::new("one", vec![vec![CellValue::Text("t".into()); 3]; 2]).with_merges(vec![
                MergedRange {
                    start_row: 0,
                    start_col: 0,
                    end_row: 0,
                    end_col: 2,
                },
            ]);
        let doc = Document::from_sheets(vec![sheet]);
        let mut v =
            Viewer::from_document(doc, Vec::new(), None, None, Box::new(NullStore)).unwrap();
        v.apply(Event::StartComment);
        type_text(&mut v, "whole title");
        v.apply(Event::Move { rows: 0, cols: 1 });
        assert_eq!(v.cursor(), (0, 1));
        assert_eq!(editor(&v), ("one!A1".to_string(), "whole title"));
    }

    #[test]
    fn merged_region_acts_as_one_cell_for_comments() {
        use crate::domain::sheet::MergedRange;
        let sheet =
            Sheet::new("one", vec![vec![CellValue::Text("t".into()); 3]; 2]).with_merges(vec![
                MergedRange {
                    start_row: 0,
                    start_col: 0,
                    end_row: 0,
                    end_col: 2,
                },
            ]);
        let doc = Document::from_sheets(vec![sheet]);

        let comments = vec![thread("one", 0, 1, false)];
        let store = RecordingStore::default();
        let log = store.log.clone();
        let mut v = Viewer::from_document(doc, comments, None, None, Box::new(store)).unwrap();
        assert!(v.thread_at_cursor().is_some(), "found from A1");
        v.apply(Event::Move { rows: 0, cols: 2 });
        assert!(v.thread_at_cursor().is_some(), "found from C1");

        v.apply(Event::StartComment);
        assert!(matches!(
            v.mode(),
            Mode::Editing {
                target: EditTarget::Reply,
                ..
            }
        ));
        v.apply(Event::CancelEdit);

        v.apply(Event::Move { rows: 1, cols: 0 });
        v.apply(Event::Move { rows: -1, cols: -1 }); // B1
        assert_eq!(v.cursor(), (0, 1));
        v.apply(Event::StartComment);
        v.apply(Event::CancelEdit);
        let doc2 = Document::from_sheets(vec![
            Sheet::new("one", vec![vec![CellValue::Text("t".into()); 3]; 2]).with_merges(vec![
                MergedRange {
                    start_row: 0,
                    start_col: 0,
                    end_row: 0,
                    end_col: 2,
                },
            ]),
        ]);
        let store2 = RecordingStore::default();
        let log2 = store2.log.clone();
        let mut v2 = Viewer::from_document(doc2, Vec::new(), None, None, Box::new(store2)).unwrap();
        v2.apply(Event::Move { rows: 0, cols: 1 }); // B1
        v2.apply(Event::StartComment);
        type_text(&mut v2, "on merge");
        v2.apply(Event::Submit);
        assert_eq!(
            log2.borrow().as_slice(),
            ["thread A1 on merge"],
            "anchored at the region's top-left, not the interior cell"
        );
        drop(log);
    }

    #[test]
    fn a_save_does_not_swallow_an_agents_concurrent_write() {
        #[derive(Clone, Default)]
        struct WritableStore {
            threads: Rc<RefCell<Vec<CommentThread>>>,
            revision: Rc<RefCell<u64>>,
        }
        impl CommentStore for WritableStore {
            fn revision(&self) -> Option<u64> {
                Some(*self.revision.borrow())
            }
            fn load(&self) -> Result<Vec<CommentThread>, StoreError> {
                Ok(self.threads.borrow().clone())
            }
            fn add_thread(
                &mut self,
                anchor: crate::domain::anchor::Anchor,
                body: &str,
                author: &str,
            ) -> Result<CommentThread, StoreError> {
                let thread = CommentThread {
                    id: format!("mine-{}", self.threads.borrow().len()),
                    anchor,
                    author: author.into(),
                    body: body.into(),
                    created_at: "2026-08-14T00:00:00Z".into(),
                    resolved: false,
                    replies: Vec::new(),
                };
                self.threads.borrow_mut().push(thread.clone());
                *self.revision.borrow_mut() += 1;
                Ok(thread)
            }
            fn add_reply(
                &mut self,
                _: &str,
                _: &str,
                _: &str,
            ) -> Result<CommentThread, StoreError> {
                Err(StoreError::Io("unused".into()))
            }
            fn resolve(&mut self, _: &str) -> Result<(), StoreError> {
                Err(StoreError::Io("unused".into()))
            }
        }

        let store = WritableStore::default();
        let shared = store.clone();
        let mut v = viewer_with(3, 3, Vec::new(), Box::new(store));

        v.apply(Event::StartComment);
        type_text(&mut v, "mine");

        shared.threads.borrow_mut().push(thread("one", 2, 2, false));
        *shared.revision.borrow_mut() += 1;

        v.apply(Event::Submit);
        v.apply(Event::Tick);

        let mut seen = v.unresolved_on_active_sheet();
        seen.sort();
        assert_eq!(
            seen,
            vec![(0, 0), (2, 2)],
            "both the user's comment and the agent's must be visible"
        );
    }

    #[test]
    fn replying_to_a_thread_resolved_mid_composition_reopens_it() {
        #[derive(Clone, Default)]
        struct ThreadStore {
            threads: Rc<RefCell<Vec<CommentThread>>>,
            revision: Rc<RefCell<u64>>,
        }
        impl CommentStore for ThreadStore {
            fn revision(&self) -> Option<u64> {
                Some(*self.revision.borrow())
            }
            fn load(&self) -> Result<Vec<CommentThread>, StoreError> {
                Ok(self.threads.borrow().clone())
            }
            fn add_thread(
                &mut self,
                _: crate::domain::anchor::Anchor,
                _: &str,
                _: &str,
            ) -> Result<CommentThread, StoreError> {
                Err(StoreError::Io("unused".into()))
            }
            fn add_reply(
                &mut self,
                thread_id: &str,
                body: &str,
                author: &str,
            ) -> Result<CommentThread, StoreError> {
                let mut threads = self.threads.borrow_mut();
                let Some(t) = threads.iter_mut().find(|t| t.id == thread_id) else {
                    return Err(StoreError::ThreadNotFound(thread_id.to_string()));
                };
                t.replies.push(Reply {
                    id: "r".into(),
                    author: author.into(),
                    body: body.into(),
                    created_at: "2026-08-15T00:00:00Z".into(),
                });
                t.resolved = false;
                *self.revision.borrow_mut() += 1;
                Ok(t.clone())
            }
            fn resolve(&mut self, thread_id: &str) -> Result<(), StoreError> {
                let mut threads = self.threads.borrow_mut();
                if let Some(t) = threads.iter_mut().find(|t| t.id == thread_id) {
                    t.resolved = true;
                }
                *self.revision.borrow_mut() += 1;
                Ok(())
            }
        }

        let store = ThreadStore::default();
        let shared = store.clone();
        shared.threads.borrow_mut().push(thread("one", 0, 0, false));
        let mut v = viewer_with(3, 3, shared.threads.borrow().clone(), Box::new(store));

        v.apply(Event::StartComment);
        type_text(&mut v, "actually, no");

        shared.threads.borrow_mut()[0].resolved = true;
        *shared.revision.borrow_mut() += 1;

        v.apply(Event::Submit);
        v.apply(Event::Tick);

        assert_eq!(
            v.unresolved_on_active_sheet(),
            vec![(0, 0)],
            "the reply must bring the thread back, marker and all"
        );
        assert!(v.thread_at_cursor().is_some_and(|t| !t.resolved));
    }

    #[test]
    fn a_reload_does_not_erase_a_save_failure() {
        #[derive(Clone, Default)]
        struct FailingStore {
            revision: Rc<RefCell<u64>>,
        }
        impl CommentStore for FailingStore {
            fn revision(&self) -> Option<u64> {
                Some(*self.revision.borrow())
            }
            fn load(&self) -> Result<Vec<CommentThread>, StoreError> {
                Ok(Vec::new())
            }
            fn add_thread(
                &mut self,
                _: crate::domain::anchor::Anchor,
                _: &str,
                _: &str,
            ) -> Result<CommentThread, StoreError> {
                Err(StoreError::Io("disk full".into()))
            }
            fn add_reply(
                &mut self,
                _: &str,
                _: &str,
                _: &str,
            ) -> Result<CommentThread, StoreError> {
                Err(StoreError::Io("disk full".into()))
            }
            fn resolve(&mut self, _: &str) -> Result<(), StoreError> {
                Err(StoreError::Io("disk full".into()))
            }
        }

        let store = FailingStore::default();
        let shared = store.clone();
        let mut v = viewer_with(3, 3, Vec::new(), Box::new(store));
        v.apply(Event::StartComment);
        type_text(&mut v, "precious");
        v.apply(Event::Submit);
        assert!(v.notice().unwrap().contains("save failed"));

        *shared.revision.borrow_mut() += 1;
        v.apply(Event::Tick);
        assert!(
            v.notice().is_some_and(|n| n.contains("save failed")),
            "the editor still holds unsaved text, so the warning must stay"
        );
        match v.mode() {
            Mode::Editing { buffer, .. } => assert_eq!(buffer, "precious"),
            other => panic!("editor should stay open, got {other:?}"),
        }
    }

    #[test]
    fn successful_retry_clears_the_failure_notice() {
        struct FlakyStore {
            failed_once: bool,
        }
        impl CommentStore for FlakyStore {
            fn load(&self) -> Result<Vec<CommentThread>, StoreError> {
                Ok(Vec::new())
            }
            fn add_thread(
                &mut self,
                anchor: crate::domain::anchor::Anchor,
                body: &str,
                author: &str,
            ) -> Result<CommentThread, StoreError> {
                if !self.failed_once {
                    self.failed_once = true;
                    return Err(StoreError::Io("disk full".into()));
                }
                Ok(CommentThread {
                    id: "t".into(),
                    anchor,
                    author: author.into(),
                    body: body.into(),
                    created_at: "2026-08-12T00:00:00Z".into(),
                    resolved: false,
                    replies: Vec::new(),
                })
            }
            fn add_reply(
                &mut self,
                _: &str,
                _: &str,
                _: &str,
            ) -> Result<CommentThread, StoreError> {
                Err(StoreError::Io("unused".into()))
            }
            fn resolve(&mut self, _: &str) -> Result<(), StoreError> {
                Err(StoreError::Io("unused".into()))
            }
        }

        let mut v = viewer_with(
            3,
            3,
            Vec::new(),
            Box::new(FlakyStore { failed_once: false }),
        );
        v.apply(Event::StartComment);
        type_text(&mut v, "hello");
        v.apply(Event::Submit);
        assert!(v.notice().is_some(), "first save fails");
        v.apply(Event::Submit);
        assert_eq!(v.notice(), None, "successful retry must clear the notice");
        assert_eq!(*v.mode(), Mode::Grid);
    }

    #[test]
    fn failed_save_keeps_the_editor_and_text() {
        let mut v = viewer(3, 3); // NullStore fails every save
        v.apply(Event::StartComment);
        type_text(&mut v, "precious text");
        v.apply(Event::Submit);
        match v.mode() {
            Mode::Editing { buffer, .. } => assert_eq!(buffer, "precious text"),
            other => panic!("editor should stay open, got {other:?}"),
        }
        assert!(v.notice().unwrap().contains("save failed"));
    }
    fn line_thread(line: u32, resolved: bool) -> CommentThread {
        let mut thread = thread("one", line, 0, resolved);
        thread.anchor = Anchor::line(line);
        thread
    }

    #[test]
    fn c_on_a_line_starts_a_thread_there() {
        let store = RecordingStore::default();
        let log = store.log.clone();
        let mut v = text_viewer_with_store("a\nb\nc\n", Vec::new(), Box::new(store));
        v.apply(Event::Move { rows: 2, cols: 0 });
        v.apply(Event::StartComment);
        match v.mode() {
            Mode::Editing { target, at, .. } => {
                assert_eq!(*target, EditTarget::NewThread);
                assert_eq!(*at, Anchor::line(2));
            }
            other => panic!("expected editing mode, got {other:?}"),
        }
        type_text(&mut v, "check this");
        v.apply(Event::Submit);
        assert_eq!(*v.mode(), Mode::Grid);
        assert_eq!(log.borrow().as_slice(), ["thread line 3 check this"]);
        assert_eq!(v.unresolved_lines(), vec![2]);
    }

    #[test]
    fn c_on_a_line_with_a_thread_continues_it_even_when_resolved() {
        let comments = vec![line_thread(1, true)];
        let store = RecordingStore::seeded(comments.clone());
        let log = store.log.clone();
        let mut v = text_viewer_with_store("a\nb\nc\n", comments, Box::new(store));
        v.apply(Event::Move { rows: 1, cols: 0 });
        v.apply(Event::StartComment);
        assert!(matches!(
            v.mode(),
            Mode::Editing {
                target: EditTarget::Reply,
                ..
            }
        ));
        type_text(&mut v, "done");
        v.apply(Event::Submit);
        assert_eq!(log.borrow().as_slice(), ["reply t-one-1-0 done"]);
    }

    #[test]
    fn the_editor_follows_the_line_cursor_and_each_line_keeps_its_draft() {
        let mut v = text_viewer_with_store("a\nb\nc\n", Vec::new(), Box::new(NullStore));
        v.apply(Event::StartComment);
        type_text(&mut v, "first");
        v.apply(Event::Bottom);
        assert_eq!(editor(&v), ("line 3".to_string(), ""));
        type_text(&mut v, "last");
        v.apply(Event::Top);
        assert_eq!(editor(&v), ("line 1".to_string(), "first"));
        v.apply(Event::CancelEdit);
        v.apply(Event::Bottom);
        v.apply(Event::StartComment);
        assert_eq!(editor(&v), ("line 3".to_string(), "last"));
    }

    #[test]
    fn a_draft_whose_line_was_cut_away_comes_along_to_the_next_line() {
        let source = text_source(&"x\n".repeat(6));
        let mut v = viewer_on_with(&source, Box::new(NullStore));
        v.apply(Event::Bottom);
        v.apply(Event::StartComment);
        type_text(&mut v, "long draft");
        source.write_text_from_outside("a\nb\n");
        v.apply(Event::Tick);
        v.apply(Event::Move { rows: -1, cols: 0 });
        assert_eq!(editor(&v), ("line 1".to_string(), "long draft"));
    }

    #[test]
    fn the_editor_stays_put_when_the_text_file_was_emptied() {
        let source = text_source("a\nb\n");
        let mut v = viewer_on_with(&source, Box::new(NullStore));
        v.apply(Event::StartComment);
        type_text(&mut v, "draft");
        source.write_text_from_outside("");
        v.apply(Event::Tick);
        v.apply(Event::Move { rows: 1, cols: 0 });
        assert_eq!(editor(&v), ("line 1".to_string(), "draft"));
    }

    #[test]
    fn a_failed_draft_on_a_renamed_sheet_comes_along_when_the_cursor_moves() {
        let cells = || vec![vec![CellValue::Number(1.0)]; 2];
        let source = SharedSource::new(vec![Sheet::new("one", cells())]);
        let mut v = viewer_on_with(&source, Box::new(LiveStore::default()));
        v.apply(Event::StartComment);
        type_text(&mut v, "precious");
        source.write_from_outside(vec![Sheet::new("renamed", cells())]);
        v.apply(Event::Tick);
        v.apply(Event::Submit);
        assert!(v.notice().is_some_and(|n| n.contains("is gone")));
        v.apply(Event::Move { rows: 1, cols: 0 });
        assert_eq!(editor(&v), ("renamed!A2".to_string(), "precious"));
    }

    #[test]
    fn a_draft_on_an_empty_sheet_stays_with_that_sheet() {
        let doc = Document::from_sheets(vec![
            Sheet::new("data", vec![vec![CellValue::Number(1.0)]]),
            Sheet::new("blank", Vec::new()),
        ]);
        let mut v =
            Viewer::from_document(doc, Vec::new(), None, None, Box::new(NullStore)).unwrap();
        v.apply(Event::NextSheet);
        v.apply(Event::StartComment);
        type_text(&mut v, "note on blank");
        v.apply(Event::NextSheet);
        assert_eq!(editor(&v), ("data!A1".to_string(), ""));
        v.apply(Event::PrevSheet);
        assert_eq!(editor(&v), ("blank!A1".to_string(), "note on blank"));
    }

    #[test]
    fn c_on_an_empty_text_document_does_nothing() {
        let mut v = text_viewer_with_store("", Vec::new(), Box::new(RecordingStore::default()));
        v.apply(Event::StartComment);
        assert_eq!(*v.mode(), Mode::Grid);
    }

    #[test]
    fn a_line_cut_away_while_typing_keeps_the_draft_and_reports_it() {
        let source = text_source("a\nb\nc\nd\n");
        let store = RecordingStore::default();
        let log = store.log.clone();
        let mut v = viewer_on_with(&source, Box::new(store));
        v.apply(Event::Bottom);
        v.apply(Event::StartComment);
        type_text(&mut v, "draft");
        source.write_text_from_outside("a\nb\n");
        v.apply(Event::Tick);
        v.apply(Event::Submit);
        assert_eq!(v.notice(), Some("save failed: line 4 is gone"));
        assert!(matches!(v.mode(), Mode::Editing { buffer, .. } if buffer == "draft"));
        assert!(log.borrow().is_empty());
    }
}
