//! The viewer state machine: one `Viewer`, one `Mode` per way of looking at
//! it. Grid navigation and reload live here; `editing` and `picker` carry
//! their modes' rules.

mod editing;
mod picker;
#[cfg(test)]
mod test_support;

pub use picker::PickerState;

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
    OpenSheetPicker,
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
    Editing {
        target: EditTarget,
        buffer: String,
    },
    /// The sheet picker: type to filter, Enter to switch, Esc to cancel.
    /// `selected` indexes the *filtered* candidate list, not the workbook.
    SheetPicker {
        query: String,
        selected: usize,
    },
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
            Mode::SheetPicker { .. } => self.apply_picker(event),
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
            // opens on the active sheet so the picker doubles as "where am I"
            Event::OpenSheetPicker => {
                self.mode = Mode::SheetPicker {
                    query: String::new(),
                    selected: self.active,
                };
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
    use std::collections::VecDeque;

    use super::test_support::{NullStore, SharedStore, thread, type_text, viewer, viewer_with};
    use super::*;

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

    #[test]
    fn a_store_without_a_revision_never_auto_reloads() {
        let store = super::test_support::RecordingStore::default(); // uses the default revision(): None
        let mut v = viewer_with(3, 3, Vec::new(), Box::new(store));
        v.apply(Event::Tick);
        assert!(v.unresolved_on_active_sheet().is_empty());
        assert_eq!(v.notice(), None);
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
