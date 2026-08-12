use std::path::Path;

use crate::domain::anchor::Anchor;
use crate::domain::comment::CommentThread;
use crate::domain::document::Document;
use crate::domain::sheet::Sheet;

use super::error::{DocumentError, FrontendError};
use super::ports::{CommentStore, DocumentSource};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    Move { rows: isize, cols: isize },
    Top,
    Bottom,
    RowStart,
    RowEnd,
    NextSheet,
    PrevSheet,
    StartComment,
    StartReply,
    Insert(char),
    Newline,
    Backspace,
    Submit,
    CancelEdit,
    ReloadComments,
    Quit,
    Noop,
}

pub trait Frontend {
    fn draw(&mut self, viewer: &Viewer) -> Result<(), FrontendError>;
    fn next_event(&mut self) -> Result<Event, FrontendError>;
}

#[derive(Debug, Clone, PartialEq)]
pub enum EditTarget {
    NewThread,
    Reply { thread_id: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Grid,
    Editing { target: EditTarget, buffer: String },
}

/// Non-empty by construction: `first` always exists.
struct Sheets {
    first: Sheet,
    rest: Vec<Sheet>,
}

impl Sheets {
    fn get(&self, index: usize) -> &Sheet {
        match index.checked_sub(1) {
            None => &self.first,
            Some(i) => self.rest.get(i).unwrap_or(&self.first),
        }
    }

    fn len(&self) -> usize {
        1 + self.rest.len()
    }
}

pub struct Viewer {
    sheets: Sheets,
    cursors: Vec<(usize, usize)>,
    active: usize,
    quit: bool,
    comments: Vec<CommentThread>,
    notice: Option<String>,
    mode: Mode,
    store: Box<dyn CommentStore>,
}

impl Viewer {
    /// A broken comment store must not block reading the document: the
    /// viewer opens without comments and carries a notice instead.
    pub fn open(
        source: &impl DocumentSource,
        store: Box<dyn CommentStore>,
        path: &Path,
    ) -> Result<Self, DocumentError> {
        let document = source.load(path)?;
        let (comments, notice) = match store.load() {
            Ok(comments) => (comments, None),
            Err(e) => (Vec::new(), Some(format!("comments unavailable: {e}"))),
        };
        Self::from_document(document, comments, notice, store)
    }

    fn from_document(
        document: Document,
        comments: Vec<CommentThread>,
        notice: Option<String>,
        store: Box<dyn CommentStore>,
    ) -> Result<Self, DocumentError> {
        let mut sheets = document.into_sheets().into_iter();
        let Some(first) = sheets.next() else {
            return Err(DocumentError::EmptyDocument);
        };
        let rest: Vec<Sheet> = sheets.collect();
        let cursors = vec![(0, 0); 1 + rest.len()];
        Ok(Self {
            sheets: Sheets { first, rest },
            cursors,
            active: 0,
            quit: false,
            comments,
            notice,
            mode: Mode::Grid,
            store,
        })
    }

    pub fn threads(&self) -> &[CommentThread] {
        &self.comments
    }

    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    pub fn mode(&self) -> &Mode {
        &self.mode
    }

    /// The thread under the cursor; unresolved threads win over resolved ones.
    pub fn thread_at_cursor(&self) -> Option<&CommentThread> {
        let (row, col) = self.cursor();
        let name = self.sheet().name();
        let at_cell: Vec<&CommentThread> = self
            .comments
            .iter()
            .filter(|t| match &t.anchor {
                Anchor::Cell {
                    sheet,
                    row: r,
                    col: c,
                } => sheet == name && *r as usize == row && *c as usize == col,
            })
            .collect();
        at_cell
            .iter()
            .find(|t| !t.resolved)
            .copied()
            .or_else(|| at_cell.first().copied())
    }

    /// (row, col) of every unresolved thread on the active sheet.
    pub fn unresolved_on_active_sheet(&self) -> Vec<(usize, usize)> {
        let active = self.sheet().name();
        self.comments
            .iter()
            .filter(|t| !t.resolved)
            .filter_map(|t| match &t.anchor {
                Anchor::Cell { sheet, row, col } if sheet == active => {
                    Some((*row as usize, *col as usize))
                }
                _ => None,
            })
            .collect()
    }

    pub fn sheet(&self) -> &Sheet {
        self.sheets.get(self.active)
    }

    pub fn sheet_names(&self) -> Vec<&str> {
        std::iter::once(self.sheets.first.name())
            .chain(self.sheets.rest.iter().map(Sheet::name))
            .collect()
    }

    pub fn sheet_count(&self) -> usize {
        self.sheets.len()
    }

    pub fn active(&self) -> usize {
        self.active
    }

    pub fn cursor(&self) -> (usize, usize) {
        self.cursors.get(self.active).copied().unwrap_or((0, 0))
    }

    pub fn wants_quit(&self) -> bool {
        self.quit
    }

    pub fn apply(&mut self, event: Event) {
        match self.mode {
            Mode::Grid => self.apply_grid(event),
            Mode::Editing { .. } => self.apply_editing(event),
        }
    }

    fn apply_grid(&mut self, event: Event) {
        let (row, col) = self.cursor();
        let max_row = self.sheet().row_count().saturating_sub(1);
        let max_col = self.sheet().col_count().saturating_sub(1);
        let next = match event {
            Event::Move { rows, cols } => (
                add_clamped(row, rows, max_row),
                add_clamped(col, cols, max_col),
            ),
            Event::Top => (0, col),
            Event::Bottom => (max_row, col),
            Event::RowStart => (row, 0),
            Event::RowEnd => (row, max_col),
            Event::NextSheet => {
                self.active = (self.active + 1) % self.sheets.len();
                return;
            }
            Event::PrevSheet => {
                self.active = (self.active + self.sheets.len() - 1) % self.sheets.len();
                return;
            }
            // Sheets model: one open thread per cell — `c` replies to it if
            // present, otherwise starts a new thread.
            Event::StartComment => {
                let target = match self.thread_at_cursor().filter(|t| !t.resolved) {
                    Some(thread) => EditTarget::Reply {
                        thread_id: thread.id.clone(),
                    },
                    None => EditTarget::NewThread,
                };
                self.mode = Mode::Editing {
                    target,
                    buffer: String::new(),
                };
                return;
            }
            Event::StartReply => {
                if let Some(thread) = self.thread_at_cursor() {
                    self.mode = Mode::Editing {
                        target: EditTarget::Reply {
                            thread_id: thread.id.clone(),
                        },
                        buffer: String::new(),
                    };
                }
                return;
            }
            Event::ReloadComments => {
                match self.store.load() {
                    Ok(comments) => {
                        self.comments = comments;
                        self.notice = None;
                    }
                    Err(e) => self.notice = Some(format!("comments unavailable: {e}")),
                }
                return;
            }
            Event::Quit => {
                self.quit = true;
                return;
            }
            _ => return,
        };
        if let Some(cursor) = self.cursors.get_mut(self.active) {
            *cursor = next;
        }
    }

    fn apply_editing(&mut self, event: Event) {
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
                let (row, col) = self.cursor();
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
            }
            Err(e) => self.notice = Some(format!("save failed: {e}")),
        }
    }
}

fn add_clamped(value: usize, delta: isize, max: usize) -> usize {
    (value as isize + delta).clamp(0, max as isize) as usize
}

pub fn run(mut viewer: Viewer, frontend: &mut impl Frontend) -> Result<(), FrontendError> {
    while !viewer.wants_quit() {
        frontend.draw(&viewer)?;
        let event = frontend.next_event()?;
        viewer.apply(event);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    use super::*;
    use crate::app::error::StoreError;
    use crate::domain::cell::CellValue;
    use crate::domain::comment::Reply;

    struct NullStore;

    impl CommentStore for NullStore {
        fn load(&self) -> Result<Vec<CommentThread>, StoreError> {
            Ok(Vec::new())
        }
        fn add_thread(&mut self, _: Anchor, _: &str, _: &str) -> Result<CommentThread, StoreError> {
            Err(StoreError("store is unavailable".into()))
        }
        fn add_reply(&mut self, _: &str, _: &str, _: &str) -> Result<CommentThread, StoreError> {
            Err(StoreError("store is unavailable".into()))
        }
        fn resolve(&mut self, _: &str) -> Result<(), StoreError> {
            Err(StoreError("store is unavailable".into()))
        }
    }

    /// Records saves; the test keeps a clone of the shared log.
    #[derive(Clone, Default)]
    struct RecordingStore {
        log: Rc<RefCell<Vec<String>>>,
    }

    impl CommentStore for RecordingStore {
        fn load(&self) -> Result<Vec<CommentThread>, StoreError> {
            Ok(Vec::new())
        }
        fn add_thread(
            &mut self,
            anchor: Anchor,
            body: &str,
            author: &str,
        ) -> Result<CommentThread, StoreError> {
            self.log
                .borrow_mut()
                .push(format!("thread {} {body}", anchor.cell_ref()));
            Ok(CommentThread {
                id: "new-thread".into(),
                anchor,
                author: author.into(),
                body: body.into(),
                created_at: "2026-08-11T00:00:00Z".into(),
                resolved: false,
                replies: Vec::new(),
            })
        }
        fn add_reply(
            &mut self,
            thread_id: &str,
            body: &str,
            author: &str,
        ) -> Result<CommentThread, StoreError> {
            self.log
                .borrow_mut()
                .push(format!("reply {thread_id} {body}"));
            Ok(CommentThread {
                id: thread_id.into(),
                anchor: Anchor::cell("one", 0, 0),
                author: "user".into(),
                body: "root".into(),
                created_at: "2026-08-11T00:00:00Z".into(),
                resolved: false,
                replies: vec![Reply {
                    id: "new-reply".into(),
                    author: author.into(),
                    body: body.into(),
                    created_at: "2026-08-11T00:00:00Z".into(),
                }],
            })
        }
        fn resolve(&mut self, _: &str) -> Result<(), StoreError> {
            Ok(())
        }
    }

    fn viewer(rows: usize, cols: usize) -> Viewer {
        viewer_with(rows, cols, Vec::new(), Box::new(NullStore))
    }

    fn viewer_with(
        rows: usize,
        cols: usize,
        comments: Vec<CommentThread>,
        store: Box<dyn CommentStore>,
    ) -> Viewer {
        let grid = vec![vec![CellValue::Number(1.0); cols]; rows];
        let doc = Document::new(vec![
            Sheet::new("one", grid),
            Sheet::new("two", vec![vec![CellValue::Bool(true)]]),
        ]);
        Viewer::from_document(doc, comments, None, store).unwrap()
    }

    fn type_text(v: &mut Viewer, text: &str) {
        for c in text.chars() {
            if c == '\n' {
                v.apply(Event::Newline);
            } else {
                v.apply(Event::Insert(c));
            }
        }
    }

    fn thread(sheet: &str, row: u32, col: u32, resolved: bool) -> CommentThread {
        CommentThread {
            id: format!("t-{sheet}-{row}-{col}"),
            anchor: Anchor::cell(sheet, row, col),
            author: "user".into(),
            body: "body".into(),
            created_at: "2026-08-11T00:00:00Z".into(),
            resolved,
            replies: Vec::new(),
        }
    }

    #[test]
    fn empty_document_is_rejected() {
        assert!(
            Viewer::from_document(Document::new(vec![]), Vec::new(), None, Box::new(NullStore))
                .is_err()
        );
    }

    #[test]
    fn unresolved_markers_follow_the_active_sheet() {
        let comments = vec![
            thread("one", 1, 1, false),
            thread("one", 2, 2, true),
            thread("two", 0, 0, false),
        ];
        let mut v = viewer_with(3, 3, comments, Box::new(NullStore));
        assert_eq!(v.unresolved_on_active_sheet(), vec![(1, 1)]);
        v.apply(Event::NextSheet);
        assert_eq!(v.unresolved_on_active_sheet(), vec![(0, 0)]);
    }

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
    fn reload_picks_up_new_comments() {
        #[derive(Clone, Default)]
        struct SharedStore {
            threads: Rc<RefCell<Vec<CommentThread>>>,
        }
        impl CommentStore for SharedStore {
            fn load(&self) -> Result<Vec<CommentThread>, StoreError> {
                Ok(self.threads.borrow().clone())
            }
            fn add_thread(
                &mut self,
                _: Anchor,
                _: &str,
                _: &str,
            ) -> Result<CommentThread, StoreError> {
                Err(StoreError("read only".into()))
            }
            fn add_reply(
                &mut self,
                _: &str,
                _: &str,
                _: &str,
            ) -> Result<CommentThread, StoreError> {
                Err(StoreError("read only".into()))
            }
            fn resolve(&mut self, _: &str) -> Result<(), StoreError> {
                Err(StoreError("read only".into()))
            }
        }

        let store = SharedStore::default();
        let shared = store.threads.clone();
        let mut v = viewer_with(3, 3, Vec::new(), Box::new(store));
        assert!(v.unresolved_on_active_sheet().is_empty());

        shared.borrow_mut().push(thread("one", 1, 1, false)); // an agent replied meanwhile
        v.apply(Event::ReloadComments);
        assert_eq!(v.unresolved_on_active_sheet(), vec![(1, 1)]);
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
                anchor: Anchor,
                body: &str,
                author: &str,
            ) -> Result<CommentThread, StoreError> {
                if !self.failed_once {
                    self.failed_once = true;
                    return Err(StoreError("disk full".into()));
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
                Err(StoreError("unused".into()))
            }
            fn resolve(&mut self, _: &str) -> Result<(), StoreError> {
                Err(StoreError("unused".into()))
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

    #[test]
    fn cursor_clamps_at_edges() {
        let mut v = viewer(3, 2);
        v.apply(Event::Move { rows: -5, cols: -5 });
        assert_eq!(v.cursor(), (0, 0));
        v.apply(Event::Move {
            rows: 100,
            cols: 100,
        });
        assert_eq!(v.cursor(), (2, 1));
    }

    #[test]
    fn jump_events() {
        let mut v = viewer(10, 4);
        v.apply(Event::Move { rows: 5, cols: 2 });
        v.apply(Event::Top);
        assert_eq!(v.cursor(), (0, 2));
        v.apply(Event::Bottom);
        assert_eq!(v.cursor(), (9, 2));
        v.apply(Event::RowStart);
        assert_eq!(v.cursor(), (9, 0));
        v.apply(Event::RowEnd);
        assert_eq!(v.cursor(), (9, 3));
    }

    #[test]
    fn sheet_switching_wraps_and_keeps_cursor_per_sheet() {
        let mut v = viewer(5, 5);
        v.apply(Event::Move { rows: 2, cols: 2 });
        v.apply(Event::NextSheet);
        assert_eq!(v.sheet().name(), "two");
        assert_eq!(v.cursor(), (0, 0));
        v.apply(Event::NextSheet);
        assert_eq!(v.sheet().name(), "one");
        assert_eq!(v.cursor(), (2, 2), "cursor remembered per sheet");
        v.apply(Event::PrevSheet);
        assert_eq!(v.sheet().name(), "two");
    }

    struct FakeFrontend {
        events: VecDeque<Event>,
        draws: usize,
    }

    impl Frontend for FakeFrontend {
        fn draw(&mut self, _: &Viewer) -> Result<(), FrontendError> {
            self.draws += 1;
            Ok(())
        }

        fn next_event(&mut self) -> Result<Event, FrontendError> {
            Ok(self.events.pop_front().unwrap_or(Event::Quit))
        }
    }

    #[test]
    fn run_draws_until_quit() {
        let mut frontend = FakeFrontend {
            events: VecDeque::from([Event::Move { rows: 1, cols: 0 }, Event::Quit]),
            draws: 0,
        };
        run(viewer(3, 3), &mut frontend).unwrap();
        assert_eq!(frontend.draws, 2);
    }
}
