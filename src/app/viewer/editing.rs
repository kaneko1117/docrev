//! The comment editor mode: building the buffer and submitting it.

use crate::domain::anchor::Anchor;

use super::{EditTarget, Event, Mode, Notice, Viewer};

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
            _ => {} // navigation is ignored while editing
        }
    }

    /// Empty input closes the editor without saving. A failed save keeps the
    /// editor open (the text is not lost) and shows a notice.
    fn submit(&mut self) {
        let Mode::Editing { target, buffer } = &self.mode else {
            return;
        };
        let body = buffer.trim();
        if body.is_empty() {
            self.mode = Mode::Grid;
            return;
        }
        let result = match target {
            EditTarget::NewThread => {
                // inside a merged region, anchor at its top-left so the
                // comment is one with the visually single cell
                let (row, col) = self.cursor();
                let (row, col) = match self.sheet().merge_at(row, col) {
                    Some(merge) => merge.anchor(),
                    None => (row, col),
                };
                let anchor = Anchor::cell(self.sheet().name(), row as u32, col as u32);
                self.store.add_thread(anchor, body, "user")
            }
            EditTarget::Reply { thread_id } => self.store.add_reply(thread_id, body, "user"),
        };
        match result {
            Ok(thread) => {
                match self.comments.iter_mut().find(|t| t.id == thread.id) {
                    Some(existing) => *existing = thread,
                    None => self.comments.push(thread),
                }
                self.mode = Mode::Grid;
                self.notice = None; // a stale "save failed" would lie now
                // deliberately NOT refreshing `revision` here: our own write
                // makes the next tick reload, which is how a write an agent
                // made while the user was typing gets picked up
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

    use super::super::test_support::{
        NullStore, RecordingStore, thread, type_text, viewer, viewer_with,
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
    fn reply_goes_to_the_thread_under_the_cursor() {
        let store = RecordingStore::default();
        let log = store.log.clone();
        let comments = vec![thread("one", 0, 0, false)];
        let mut v = viewer_with(3, 3, comments, Box::new(store));
        v.apply(Event::StartReply);
        type_text(&mut v, "done");
        v.apply(Event::Submit);
        assert_eq!(log.borrow().as_slice(), ["reply t-one-0-0 done"]);
        let updated = v.thread_at_cursor().unwrap();
        assert_eq!(updated.replies.len(), 1);
    }

    #[test]
    fn reply_without_a_thread_is_ignored() {
        let mut v = viewer(3, 3);
        v.apply(Event::StartReply);
        assert_eq!(*v.mode(), Mode::Grid);
    }

    #[test]
    fn c_on_an_open_thread_replies_instead_of_forking() {
        let comments = vec![thread("one", 0, 0, false)];
        let mut v = viewer_with(3, 3, comments, Box::new(NullStore));
        v.apply(Event::StartComment);
        match v.mode() {
            Mode::Editing {
                target: EditTarget::Reply { thread_id },
                ..
            } => assert_eq!(thread_id, "t-one-0-0"),
            other => panic!("expected reply mode, got {other:?}"),
        }
    }

    #[test]
    fn c_on_a_resolved_thread_starts_a_new_thread() {
        let comments = vec![thread("one", 0, 0, true)];
        let mut v = viewer_with(3, 3, comments, Box::new(NullStore));
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
        let doc = Document::new(vec![sheet]);

        // a thread anchored on an interior cell (B1) must be discoverable
        // from anywhere in the region
        let comments = vec![thread("one", 0, 1, false)];
        let store = RecordingStore::default();
        let log = store.log.clone();
        let mut v = Viewer::from_document(doc, comments, None, None, Box::new(store)).unwrap();
        assert!(v.thread_at_cursor().is_some(), "found from A1");
        v.apply(Event::Move { rows: 0, cols: 2 });
        assert!(v.thread_at_cursor().is_some(), "found from C1");

        // `c` on the interior cell replies to that thread instead of forking
        v.apply(Event::StartComment);
        assert!(matches!(
            v.mode(),
            Mode::Editing {
                target: EditTarget::Reply { .. },
                ..
            }
        ));
        v.apply(Event::CancelEdit);

        // a new thread from an interior cell anchors at the region's top-left
        v.apply(Event::Move { rows: 1, cols: 0 });
        v.apply(Event::Move { rows: -1, cols: -1 }); // B1 (interior)
        assert_eq!(v.cursor(), (0, 1));
        v.apply(Event::StartReply);
        v.apply(Event::CancelEdit);
        let doc2 = Document::new(vec![
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
        v2.apply(Event::Move { rows: 0, cols: 1 }); // B1, interior, no thread yet
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

    /// The review's headline finding: an agent writing while the user typed
    /// used to be stamped as "already seen" by the save, hiding it forever.
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

        // the user starts typing; no tick fires while keys keep arriving
        v.apply(Event::StartComment);
        type_text(&mut v, "mine");

        // an agent writes to the same sidecar mid-typing
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

    /// The user answers a thread the agent resolved mid-composition.
    /// The reply must come back into view rather than vanish.
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
                t.resolved = false; // the store's contract
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

        v.apply(Event::StartReply);
        type_text(&mut v, "actually, no");

        // the agent resolves it while the user is still typing
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

        // an unrelated agent write must not make the warning disappear
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
