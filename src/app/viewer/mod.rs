mod editing;
mod matching;
mod mouse;
mod picker;
mod search;
#[cfg(test)]
mod test_support;

pub use picker::PickerState;
pub use search::SearchState;

use std::path::{Path, PathBuf};

use crate::domain::anchor::Anchor;
use crate::domain::comment::CommentThread;
use crate::domain::document::Document;
use crate::domain::sheet::Sheet;
use crate::domain::workbook_comment::WorkbookComment;

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
    OpenSearch,
    OpenNotes,
    /// A click, or the press that may begin a drag.
    SelectCell {
        row: usize,
        col: usize,
    },
    SelectSheet(usize),
    DragTo {
        row: usize,
        col: usize,
    },
    /// `copy` is true after a real drag, not a plain click.
    DragEnd {
        copy: bool,
    },
    StartComment,
    StartReply,
    Insert(char),
    Newline,
    Backspace,
    Submit,
    CancelEdit,
    /// Fired when no input arrived for a while; triggers reload checks.
    Tick,
    Quit,
    Noop,
}

pub trait Frontend {
    fn draw(&mut self, viewer: &Viewer) -> Result<(), FrontendError>;
    fn next_event(&mut self) -> Result<Event, FrontendError>;
    /// The default drops the text.
    fn copy(&mut self, _text: &str) -> Result<(), FrontendError> {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum EditTarget {
    NewThread,
    Reply { thread_id: String },
}

#[derive(Debug, Clone, PartialEq)]
enum Notice {
    Reload(String),
    /// The file changed but could not be re-read; the grid shows the previous load.
    Document(String),
    Save(String),
    /// Cleared by the next input.
    Copy(String),
}

impl Notice {
    fn text(&self) -> &str {
        match self {
            Notice::Reload(text)
            | Notice::Document(text)
            | Notice::Save(text)
            | Notice::Copy(text) => text,
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
    /// `selected` indexes the filtered candidate list, not the workbook.
    SheetPicker {
        query: String,
        selected: usize,
    },
    /// Esc returns to `origin`; `matches` is recomputed on document reload.
    Search {
        query: String,
        origin: (usize, usize),
        matches: Vec<(usize, usize)>,
        index: usize,
    },
    Notes {
        scroll: usize,
    },
}

/// Non-empty by construction.
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
    revision: Option<u64>,
    /// `None` disables document auto-reload.
    source: Option<(Box<dyn DocumentSource>, PathBuf)>,
    doc_revision: Option<u64>,
    /// Set when the last document reload failed; kept apart from `notice`.
    doc_stale: Option<String>,
    /// (press cell, current cell), only while the button is down.
    selection: Option<((usize, usize), (usize, usize))>,
    /// TSV waiting for the frontend.
    copy_request: Option<String>,
}

impl Viewer {
    /// A broken comment store yields a viewer without comments plus a notice.
    pub fn open(
        source: Box<dyn DocumentSource>,
        store: Box<dyn CommentStore>,
        path: &Path,
    ) -> Result<Self, DocumentError> {
        // revisions are read before loading so a concurrent write looks newer
        let doc_revision = source.revision(path);
        let document = source.load(path)?;
        let revision = store.revision();
        let (comments, notice) = match store.load() {
            Ok(comments) => (comments, None),
            Err(e) => (
                Vec::new(),
                Some(Notice::Reload(format!("comments unavailable: {e}"))),
            ),
        };
        let mut viewer = Self::from_document(document, comments, notice, revision, store)?;
        viewer.source = Some((source, path.to_path_buf()));
        viewer.doc_revision = doc_revision;
        Ok(viewer)
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
            source: None,
            doc_revision: None,
            doc_stale: None,
            selection: None,
            copy_request: None,
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

    /// Unresolved threads win; a merged region counts as one cell.
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

    /// (row, col) per unresolved thread.
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

    /// (press cell, current cell).
    pub fn selection(&self) -> Option<((usize, usize), (usize, usize))> {
        self.selection
    }

    /// (comments, scroll offset) while the notes dialog is open.
    pub fn notes_state(&self) -> Option<(Vec<&WorkbookComment>, usize)> {
        let Mode::Notes { scroll } = &self.mode else {
            return None;
        };
        let (row, col) = self.cursor();
        Some((self.sheet().workbook_comments_at(row, col), *scroll))
    }

    pub fn has_notes_at_cursor(&self) -> bool {
        let (row, col) = self.cursor();
        !self.sheet().workbook_comments_at(row, col).is_empty()
    }

    pub fn workbook_comment_cells(&self) -> Vec<(usize, usize)> {
        self.sheet()
            .workbook_comments()
            .iter()
            .map(|c| (c.row, c.col))
            .collect()
    }

    pub fn take_copy_request(&mut self) -> Option<String> {
        self.copy_request.take()
    }

    pub fn apply(&mut self, event: Event) {
        if event == Event::Tick {
            self.reload_comments_if_changed();
            self.reload_document_if_changed();
            return;
        }
        // Noop is bare pointer motion, which must not retire the copy notice
        if event != Event::Noop && matches!(self.notice, Some(Notice::Copy(_))) {
            self.notice = None;
        }
        // any event outside the drag dissolves the selection
        if !matches!(
            event,
            Event::SelectCell { .. } | Event::DragTo { .. } | Event::DragEnd { .. } | Event::Noop
        ) {
            self.selection = None;
        }
        // a click closes any prompt (discarding a draft) and then acts
        if Self::is_mouse(event) && !matches!(self.mode, Mode::Grid) {
            self.mode = Mode::Grid;
        }
        match self.mode {
            Mode::Grid => self.apply_grid(event),
            Mode::Editing { .. } => self.apply_editing(event),
            Mode::SheetPicker { .. } => self.apply_picker(event),
            Mode::Search { .. } => self.apply_search(event),
            Mode::Notes { .. } => self.apply_notes(event),
        }
    }

    fn apply_notes(&mut self, event: Event) {
        let Mode::Notes { scroll } = &mut self.mode else {
            return;
        };
        match event {
            Event::Move { rows, .. } => {
                // not clamped here: only the renderer knows the line count
                *scroll = scroll.saturating_add_signed(rows);
            }
            Event::CancelEdit | Event::Submit | Event::OpenNotes => self.mode = Mode::Grid,
            Event::Quit => self.quit = true,
            _ => {}
        }
    }

    fn is_mouse(event: Event) -> bool {
        matches!(
            event,
            Event::SelectCell { .. }
                | Event::SelectSheet(_)
                | Event::DragTo { .. }
                | Event::DragEnd { .. }
        )
    }

    fn reload_comments_if_changed(&mut self) {
        let current = self.store.revision();
        if current.is_none() || current == self.revision {
            return;
        }
        self.revision = current;
        match self.store.load() {
            Ok(comments) => {
                self.comments = comments;
                // a save-failure notice must survive a reload
                if matches!(self.notice, Some(Notice::Reload(_))) {
                    self.notice = None;
                }
            }
            Err(e) => self.notice = Some(Notice::Reload(format!("comments unavailable: {e}"))),
        }
    }

    /// A failed load keeps the current grid (a tick can land mid-write); the
    /// revision is latched either way so a broken file is not re-parsed per tick.
    fn reload_document_if_changed(&mut self) {
        let Some((source, path)) = &self.source else {
            return;
        };
        let current = source.revision(path);
        if current.is_some() && current != self.doc_revision {
            self.doc_revision = current;
            match source.load(path) {
                Ok(document) => self.replace_document(document),
                Err(e) => self.doc_stale = Some(format!("document unavailable: {e}")),
            }
        }
        // only takes the notice slot when free; never clobbers a save failure
        match &self.doc_stale {
            Some(reason) => {
                if matches!(self.notice, None | Some(Notice::Document(_))) {
                    self.notice = Some(Notice::Document(reason.clone()));
                }
            }
            None => {
                if matches!(self.notice, Some(Notice::Document(_))) {
                    self.notice = None;
                }
            }
        }
    }

    /// Active sheet and cursors carry over by sheet name, clamped; a document
    /// with no sheets keeps the old view.
    fn replace_document(&mut self, document: Document) {
        let mut incoming = document.into_sheets().into_iter();
        let Some(first) = incoming.next() else {
            self.doc_stale = Some(format!(
                "document unavailable: {}",
                DocumentError::EmptyDocument
            ));
            return;
        };
        let new = Sheets {
            first,
            rest: incoming.collect(),
        };
        let old_names: Vec<String> = (0..self.sheets.len())
            .map(|i| self.sheets.get(i).name().to_string())
            .collect();
        let cursors: Vec<(usize, usize)> = (0..new.len())
            .map(|i| {
                let sheet = new.get(i);
                let (row, col) = old_names
                    .iter()
                    .position(|name| name == sheet.name())
                    .and_then(|old| self.cursors.get(old).copied())
                    .unwrap_or((0, 0));
                (
                    row.min(sheet.row_count().saturating_sub(1)),
                    col.min(sheet.col_count().saturating_sub(1)),
                )
            })
            .collect();
        let active_name = old_names.get(self.active).cloned().unwrap_or_default();
        self.active = (0..new.len())
            .position(|i| new.get(i).name() == active_name)
            .unwrap_or(0);
        self.sheets = new;
        self.cursors = cursors;
        self.selection = None;
        self.doc_stale = None;
        self.refresh_mode_after_reload();
    }

    fn refresh_mode_after_reload(&mut self) {
        match &self.mode {
            Mode::Search { .. } => self.refresh_search(),
            Mode::SheetPicker { .. } => self.clamp_picker_selection(),
            Mode::Grid | Mode::Editing { .. } | Mode::Notes { .. } => {}
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
            Event::OpenSheetPicker => {
                self.mode = Mode::SheetPicker {
                    query: String::new(),
                    selected: self.active,
                };
                return;
            }
            Event::OpenSearch => {
                self.mode = Mode::Search {
                    query: String::new(),
                    origin: (row, col),
                    matches: Vec::new(),
                    index: 0,
                };
                return;
            }
            Event::OpenNotes => {
                if !self.sheet().workbook_comments_at(row, col).is_empty() {
                    self.mode = Mode::Notes { scroll: 0 };
                }
                return;
            }
            // one open thread per cell: reply if present, else start one
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
            Event::SelectCell { .. }
            | Event::SelectSheet(_)
            | Event::DragTo { .. }
            | Event::DragEnd { .. } => {
                self.apply_mouse(event);
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
        if let Some(text) = viewer.take_copy_request() {
            frontend.copy(&text)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::test_support::{
        NullStore, SharedSource, SharedStore, thread, type_text, viewer, viewer_on, viewer_on_with,
        viewer_with,
    };
    use super::*;
    use crate::domain::cell::CellValue;

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
        let store = super::test_support::RecordingStore::default(); // revision() is None
        let mut v = viewer_with(3, 3, Vec::new(), Box::new(store));
        v.apply(Event::Tick);
        assert!(v.unresolved_on_active_sheet().is_empty());
        assert_eq!(v.notice(), None);
    }

    fn one_cell(name: &str, text: &str) -> Sheet {
        Sheet::new(name, vec![vec![CellValue::Text(text.into())]])
    }

    #[test]
    fn a_tick_reloads_the_document_when_its_file_changes() {
        let source = SharedSource::new(vec![one_cell("one", "old")]);
        let mut v = viewer_on(&source);
        source.write_from_outside(vec![one_cell("one", "new")]);
        assert_eq!(v.sheet().cell(0, 0).display_text(), "old", "not yet");
        v.apply(Event::Tick);
        assert_eq!(v.sheet().cell(0, 0).display_text(), "new");
        assert_eq!(v.notice(), None);
    }

    #[test]
    fn a_document_reload_keeps_the_place_by_sheet_name() {
        let source = SharedSource::new(vec![
            Sheet::new("one", vec![vec![CellValue::Number(1.0); 3]; 3]),
            one_cell("two", "b"),
        ]);
        let mut v = viewer_on(&source);
        v.apply(Event::Move { rows: 2, cols: 2 });
        v.apply(Event::NextSheet);
        source.write_from_outside(vec![one_cell("two", "b"), one_cell("one", "shrunk")]);
        v.apply(Event::Tick);
        assert_eq!(v.sheet().name(), "two", "active follows the name");
        v.apply(Event::NextSheet);
        assert_eq!(v.sheet().name(), "one");
        assert_eq!(v.cursor(), (0, 0), "cursor clamped into the smaller sheet");
    }

    #[test]
    fn a_document_reload_falls_back_to_the_first_sheet_when_the_active_one_is_gone() {
        let source = SharedSource::new(vec![one_cell("one", "a"), one_cell("two", "b")]);
        let mut v = viewer_on(&source);
        v.apply(Event::NextSheet);
        assert_eq!(v.sheet().name(), "two");
        source.write_from_outside(vec![one_cell("uno", "a")]);
        v.apply(Event::Tick);
        assert_eq!(v.sheet().name(), "uno");
        assert_eq!(v.cursor(), (0, 0));
    }

    #[test]
    fn a_failed_document_reload_keeps_the_view_until_the_writer_finishes() {
        let source = SharedSource::new(vec![one_cell("one", "old")]);
        let mut v = viewer_on(&source);
        *source.broken.borrow_mut() = true;
        source.write_from_outside(vec![one_cell("one", "new")]);
        v.apply(Event::Tick);
        assert_eq!(v.sheet().cell(0, 0).display_text(), "old", "old view kept");
        assert_eq!(v.notice(), Some("document unavailable: mid-write"));
        let parses = *source.loads.borrow();
        v.apply(Event::Tick);
        assert_eq!(
            *source.loads.borrow(),
            parses,
            "an unchanged broken file is not re-parsed on every tick"
        );
        *source.broken.borrow_mut() = false;
        source.write_from_outside(vec![one_cell("one", "new")]);
        v.apply(Event::Tick);
        assert_eq!(v.sheet().cell(0, 0).display_text(), "new");
        assert_eq!(v.notice(), None, "recovery clears the warning");
    }

    #[test]
    fn a_document_failure_does_not_clobber_a_save_warning() {
        let source = SharedSource::new(vec![one_cell("one", "old")]);
        let mut v = viewer_on(&source);
        v.apply(Event::StartComment);
        type_text(&mut v, "precious");
        v.apply(Event::Submit); // NullStore refuses every write
        assert!(v.notice().is_some_and(|n| n.starts_with("save failed")));
        *source.broken.borrow_mut() = true;
        source.write_from_outside(vec![one_cell("one", "new")]);
        v.apply(Event::Tick);
        assert!(
            v.notice().is_some_and(|n| n.starts_with("save failed")),
            "the warning guarding the unsaved draft survives"
        );
        assert!(matches!(v.mode(), Mode::Editing { buffer, .. } if buffer == "precious"));
    }

    #[test]
    fn a_stale_document_warning_resurfaces_after_other_notices() {
        let source = SharedSource::new(vec![one_cell("one", "old")]);
        let store = SharedStore::default();
        let mut v = viewer_on_with(&source, Box::new(store.clone()));
        source.write_from_outside(Vec::new());
        v.apply(Event::Tick);
        assert_eq!(
            v.notice(),
            Some("document unavailable: document has no sheets")
        );
        *store.broken.borrow_mut() = true;
        store.write_from_outside(Vec::new());
        v.apply(Event::Tick);
        assert_eq!(v.notice(), Some("comments unavailable: invalid sidecar"));
        *store.broken.borrow_mut() = false;
        store.write_from_outside(Vec::new());
        v.apply(Event::Tick);
        assert_eq!(
            v.notice(),
            Some("document unavailable: document has no sheets"),
            "the staleness warning comes back"
        );
    }

    #[test]
    fn a_source_without_a_revision_never_reloads_the_document() {
        let source = SharedSource::new(vec![one_cell("one", "old")]);
        *source.revision.borrow_mut() = None;
        let mut v = viewer_on(&source);
        *source.sheets.borrow_mut() = vec![one_cell("one", "new")];
        v.apply(Event::Tick);
        assert_eq!(v.sheet().cell(0, 0).display_text(), "old");
    }

    #[test]
    fn a_reload_that_lost_every_sheet_keeps_the_old_view() {
        let source = SharedSource::new(vec![one_cell("one", "old")]);
        let mut v = viewer_on(&source);
        source.write_from_outside(Vec::new());
        v.apply(Event::Tick);
        assert_eq!(v.sheet().cell(0, 0).display_text(), "old");
        assert_eq!(
            v.notice(),
            Some("document unavailable: document has no sheets")
        );
        source.write_from_outside(vec![one_cell("one", "back")]);
        v.apply(Event::Tick);
        assert_eq!(v.sheet().cell(0, 0).display_text(), "back");
        assert_eq!(v.notice(), None);
    }

    #[test]
    fn a_document_reload_leaves_an_editing_draft_untouched() {
        let source = SharedSource::new(vec![one_cell("one", "old")]);
        let mut v = viewer_on(&source);
        v.apply(Event::StartComment);
        type_text(&mut v, "draft");
        source.write_from_outside(vec![one_cell("one", "new")]);
        v.apply(Event::Tick);
        assert_eq!(v.sheet().cell(0, 0).display_text(), "new");
        assert!(
            matches!(v.mode(), Mode::Editing { buffer, .. } if buffer == "draft"),
            "the draft survives the reload"
        );
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

    #[test]
    fn notes_open_only_where_the_workbook_has_comments() {
        use crate::domain::cell::CellValue;
        let sheet = Sheet::new(
            "s",
            vec![vec![CellValue::Number(1.0), CellValue::Number(2.0)]],
        )
        .with_workbook_comments(vec![WorkbookComment {
            row: 0,
            col: 1,
            author: "田中".into(),
            body: "メモ".into(),
            resolved: false,
            replies: Vec::new(),
        }]);
        let mut v = Viewer::from_document(
            Document::new(vec![sheet]),
            Vec::new(),
            None,
            None,
            Box::new(NullStore),
        )
        .unwrap();

        v.apply(Event::OpenNotes);
        assert_eq!(*v.mode(), Mode::Grid, "no notes on A1: nothing opens");

        v.apply(Event::Move { rows: 0, cols: 1 });
        v.apply(Event::OpenNotes);
        let (comments, scroll) = v.notes_state().expect("the dialog is open");
        assert_eq!(comments.len(), 1);
        assert_eq!(scroll, 0);

        v.apply(Event::Move { rows: 3, cols: 0 });
        assert_eq!(v.notes_state().unwrap().1, 3, "arrows scroll");
        v.apply(Event::Move { rows: -5, cols: 0 });
        assert_eq!(v.notes_state().unwrap().1, 0, "floored at the top");

        v.apply(Event::CancelEdit);
        assert_eq!(*v.mode(), Mode::Grid, "Esc closes");

        v.apply(Event::OpenNotes);
        v.apply(Event::SelectCell { row: 0, col: 0 });
        assert_eq!(*v.mode(), Mode::Grid);
        assert_eq!(v.cursor(), (0, 0));
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
