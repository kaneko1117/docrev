use crate::app::comments;
use crate::domain::anchor::Anchor;

use super::{Event, Mode, Notice, Viewer};

impl Viewer {
    pub(super) fn apply_editing(&mut self, event: Event) {
        let Mode::Editing { buffer, .. } = &mut self.mode else {
            return;
        };
        match event {
            Event::Insert(c) => buffer.push(c),
            Event::Newline => buffer.push('\n'),
            Event::Backspace => {
                buffer.pop();
            }
            Event::CancelEdit => self.mode = Mode::Grid,
            Event::Submit => self.submit(),
            _ => {}
        }
    }

    /// Empty input closes without saving; a failed save keeps the editor open. The thread is
    /// chosen at save time, so one that appeared on the cell while typing is continued, not forked.
    fn submit(&mut self) {
        let Mode::Editing { at, buffer, .. } = &self.mode else {
            return;
        };
        let body = buffer.trim().to_string();
        if body.is_empty() {
            self.mode = Mode::Grid;
            return;
        }
        let Anchor::Cell { sheet, row, col } = at else {
            self.notice = Some(Notice::Save("save failed: not a cell".into()));
            return;
        };
        let (name, row, col) = (sheet.clone(), *row as usize, *col as usize);
        let Some(index) = self.sheet_names().iter().position(|n| *n == name) else {
            self.notice = Some(Notice::Save(format!("save failed: sheet {name:?} is gone")));
            return;
        };
        let sheet = self.sheets.get(index);
        let result = comments::comment_on_cell(self.store.as_mut(), sheet, row, col, &body, "user");
        match result {
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
            Err(e) => self.notice = Some(Notice::Save(format!("save failed: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::app::error::StoreError;
    use crate::app::ports::CommentStore;
    use crate::domain::cell::CellValue;
    use crate::domain::comment::{CommentThread, Reply};
    use crate::domain::document::Document;
    use crate::domain::sheet::Sheet;

    use super::super::EditTarget;
    use super::super::test_support::{
        LiveStore, NullStore, RecordingStore, SharedSource, thread, type_text, viewer,
        viewer_on_with, viewer_with,
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

    #[test]
    fn escape_cancels_without_saving() {
        let store = RecordingStore::default();
        let log = store.log.clone();
        let mut v = viewer_with(3, 3, Vec::new(), Box::new(store));
        v.apply(Event::StartComment);
        type_text(&mut v, "draft");
        v.apply(Event::CancelEdit);
        assert_eq!(*v.mode(), Mode::Grid);
        assert!(log.borrow().is_empty());
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
    fn navigation_is_ignored_while_editing() {
        let mut v = viewer(3, 3);
        v.apply(Event::StartComment);
        v.apply(Event::Move { rows: 1, cols: 1 });
        assert_eq!(v.cursor(), (0, 0));
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
}
