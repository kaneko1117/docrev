use std::path::Path;

use crate::domain::anchor::Anchor;
use crate::domain::comment::CommentThread;
use crate::domain::document::Document;
use crate::domain::sheet::Sheet;

use super::error::{DocumentError, FrontendError};
use super::ports::{CommentStore, DocumentSource};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    Move {
        rows: isize,
        cols: isize,
    },
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
    /// The frontend saw no input for a while — a chance to notice outside
    /// changes (an agent replying) without any keypress.
    Tick,
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

/// Why a notice is on screen. A reload must not clear a save failure: the
/// user would take the empty status bar for a successful save and press Esc,
/// losing the text the editor is still holding.
#[derive(Debug, Clone, PartialEq)]
enum Notice {
    Reload(String),
    Save(String),
}

impl Notice {
    fn text(&self) -> &str {
        match self {
            Notice::Reload(text) | Notice::Save(text) => text,
        }
    }
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
    notice: Option<Notice>,
    mode: Mode,
    store: Box<dyn CommentStore>,
    /// Store revision the loaded comments came from.
    revision: Option<u64>,
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
        // read the revision first: a write that lands while we are loading
        // must look newer than what we loaded, not older
        let revision = store.revision();
        let (comments, notice) = match store.load() {
            Ok(comments) => (comments, None),
            Err(e) => (
                Vec::new(),
                Some(Notice::Reload(format!("comments unavailable: {e}"))),
            ),
        };
        Self::from_document(document, comments, notice, revision, store)
    }

    fn from_document(
        document: Document,
        comments: Vec<CommentThread>,
        notice: Option<Notice>,
        revision: Option<u64>,
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
            revision,
            store,
        })
    }

    pub fn threads(&self) -> &[CommentThread] {
        &self.comments
    }

    pub fn notice(&self) -> Option<&str> {
        self.notice.as_ref().map(Notice::text)
    }

    pub fn mode(&self) -> &Mode {
        &self.mode
    }

    /// The thread under the cursor; unresolved threads win over resolved
    /// ones. Inside a merged region the whole region counts as one cell, so
    /// threads anchored to any of its cells are found.
    pub fn thread_at_cursor(&self) -> Option<&CommentThread> {
        let (row, col) = self.cursor();
        let sheet = self.sheet();
        let name = sheet.name();
        let merge = sheet.merge_at(row, col);
        let in_region = |r: usize, c: usize| match merge {
            Some(m) => m.contains(r, c),
            None => (r, c) == (row, col),
        };
        let at_cell: Vec<&CommentThread> = self
            .comments
            .iter()
            .filter(|t| match &t.anchor {
                Anchor::Cell {
                    sheet,
                    row: r,
                    col: c,
                } => sheet == name && in_region(*r as usize, *c as usize),
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
        // outside changes are picked up in either mode; typing is untouched
        if event == Event::Tick {
            self.reload_if_changed();
            return;
        }
        match self.mode {
            Mode::Grid => self.apply_grid(event),
            Mode::Editing { .. } => self.apply_editing(event),
        }
    }

    /// Re-reads the comments when the store reports a new revision.
    fn reload_if_changed(&mut self) {
        let current = self.store.revision();
        if current.is_none() || current == self.revision {
            return;
        }
        self.revision = current;
        match self.store.load() {
            Ok(comments) => {
                self.comments = comments;
                // a save failure must survive: the editor still holds that text
                if matches!(self.notice, Some(Notice::Reload(_))) {
                    self.notice = None;
                }
            }
            Err(e) => self.notice = Some(Notice::Reload(format!("comments unavailable: {e}"))),
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
        // mirror `open`: the revision is read before the comments are loaded
        let revision = store.revision();
        Viewer::from_document(doc, comments, None, revision, store).unwrap()
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
            Viewer::from_document(
                Document::new(vec![]),
                Vec::new(),
                None,
                None,
                Box::new(NullStore)
            )
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

    /// A store an "agent" can edit behind the viewer's back.
    #[derive(Clone, Default)]
    struct SharedStore {
        threads: Rc<RefCell<Vec<CommentThread>>>,
        revision: Rc<RefCell<u64>>,
        loads: Rc<RefCell<usize>>,
        broken: Rc<RefCell<bool>>,
    }

    impl SharedStore {
        /// Simulates an outside write: new content plus a new revision.
        fn write_from_outside(&self, threads: Vec<CommentThread>) {
            *self.threads.borrow_mut() = threads;
            *self.revision.borrow_mut() += 1;
        }
    }

    impl CommentStore for SharedStore {
        fn revision(&self) -> Option<u64> {
            Some(*self.revision.borrow())
        }
        fn load(&self) -> Result<Vec<CommentThread>, StoreError> {
            *self.loads.borrow_mut() += 1;
            if *self.broken.borrow() {
                return Err(StoreError("invalid sidecar".into()));
            }
            Ok(self.threads.borrow().clone())
        }
        fn add_thread(&mut self, _: Anchor, _: &str, _: &str) -> Result<CommentThread, StoreError> {
            Err(StoreError("read only".into()))
        }
        fn add_reply(&mut self, _: &str, _: &str, _: &str) -> Result<CommentThread, StoreError> {
            Err(StoreError("read only".into()))
        }
        fn resolve(&mut self, _: &str) -> Result<(), StoreError> {
            Err(StoreError("read only".into()))
        }
    }

    #[test]
    fn a_tick_picks_up_an_agents_reply() {
        let store = SharedStore::default();
        let shared = store.clone();
        let mut v = viewer_with(3, 3, Vec::new(), Box::new(store));
        assert!(v.unresolved_on_active_sheet().is_empty());

        shared.write_from_outside(vec![thread("one", 1, 1, false)]);
        v.apply(Event::Tick);
        assert_eq!(v.unresolved_on_active_sheet(), vec![(1, 1)]);
    }

    #[test]
    fn ticks_do_not_re_read_an_unchanged_store() {
        let store = SharedStore::default();
        let shared = store.clone();
        let mut v = viewer_with(3, 3, Vec::new(), Box::new(store));
        let before = *shared.loads.borrow();
        for _ in 0..10 {
            v.apply(Event::Tick);
        }
        assert_eq!(*shared.loads.borrow(), before, "idle ticks stay cheap");
    }

    #[test]
    fn a_tick_reloads_while_editing_without_touching_the_buffer() {
        let store = SharedStore::default();
        let shared = store.clone();
        let mut v = viewer_with(3, 3, Vec::new(), Box::new(store));
        v.apply(Event::StartComment);
        type_text(&mut v, "half-typed");

        shared.write_from_outside(vec![thread("one", 2, 2, false)]);
        v.apply(Event::Tick);

        assert_eq!(v.unresolved_on_active_sheet(), vec![(2, 2)]);
        match v.mode() {
            Mode::Editing { buffer, .. } => assert_eq!(buffer, "half-typed"),
            other => panic!("still editing, got {other:?}"),
        }
    }

    #[test]
    fn a_failed_reload_surfaces_a_notice() {
        let store = SharedStore::default();
        let shared = store.clone();
        let mut v = viewer_with(3, 3, Vec::new(), Box::new(store));
        *shared.broken.borrow_mut() = true;
        *shared.revision.borrow_mut() += 1;
        v.apply(Event::Tick);
        assert!(v.notice().unwrap().contains("comments unavailable"));
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
                anchor: Anchor,
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
                Err(StoreError("unused".into()))
            }
            fn resolve(&mut self, _: &str) -> Result<(), StoreError> {
                Err(StoreError("unused".into()))
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
                _: Anchor,
                _: &str,
                _: &str,
            ) -> Result<CommentThread, StoreError> {
                Err(StoreError("disk full".into()))
            }
            fn add_reply(
                &mut self,
                _: &str,
                _: &str,
                _: &str,
            ) -> Result<CommentThread, StoreError> {
                Err(StoreError("disk full".into()))
            }
            fn resolve(&mut self, _: &str) -> Result<(), StoreError> {
                Err(StoreError("disk full".into()))
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
    fn a_store_without_a_revision_never_auto_reloads() {
        let store = RecordingStore::default(); // uses the default revision(): None
        let mut v = viewer_with(3, 3, Vec::new(), Box::new(store));
        v.apply(Event::Tick);
        assert!(v.unresolved_on_active_sheet().is_empty());
        assert_eq!(v.notice(), None);
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
