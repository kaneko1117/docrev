//! Store fakes and builders shared by the viewer's test modules.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use crate::app::error::{LoadError, StoreError};
use crate::app::ports::{CommentStore, DocumentSource};
use crate::domain::anchor::Anchor;
use crate::domain::cell::CellValue;
use crate::domain::comment::{CommentThread, Reply};
use crate::domain::document::Document;
use crate::domain::sheet::Sheet;

use super::{Event, Viewer};

pub(crate) struct NullStore;

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
pub(crate) struct RecordingStore {
    pub(crate) log: Rc<RefCell<Vec<String>>>,
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

/// A store an "agent" can edit behind the viewer's back.
#[derive(Clone, Default)]
pub(crate) struct SharedStore {
    pub(crate) threads: Rc<RefCell<Vec<CommentThread>>>,
    pub(crate) revision: Rc<RefCell<u64>>,
    pub(crate) loads: Rc<RefCell<usize>>,
    pub(crate) broken: Rc<RefCell<bool>>,
}

impl SharedStore {
    /// Simulates an outside write: new content plus a new revision.
    pub(crate) fn write_from_outside(&self, threads: Vec<CommentThread>) {
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

/// A document an "agent" can rewrite behind the viewer's back.
#[derive(Clone)]
pub(crate) struct SharedSource {
    pub(crate) sheets: Rc<RefCell<Vec<Sheet>>>,
    pub(crate) revision: Rc<RefCell<Option<u64>>>,
    pub(crate) loads: Rc<RefCell<usize>>,
    pub(crate) broken: Rc<RefCell<bool>>,
}

impl SharedSource {
    pub(crate) fn new(sheets: Vec<Sheet>) -> Self {
        Self {
            sheets: Rc::new(RefCell::new(sheets)),
            revision: Rc::new(RefCell::new(Some(1))),
            loads: Rc::new(RefCell::new(0)),
            broken: Rc::new(RefCell::new(false)),
        }
    }

    /// Simulates an outside write: new content plus a new revision.
    pub(crate) fn write_from_outside(&self, sheets: Vec<Sheet>) {
        *self.sheets.borrow_mut() = sheets;
        if let Some(revision) = self.revision.borrow_mut().as_mut() {
            *revision += 1;
        }
    }
}

impl DocumentSource for SharedSource {
    fn load(&self, _: &Path) -> Result<Document, LoadError> {
        *self.loads.borrow_mut() += 1;
        if *self.broken.borrow() {
            return Err(LoadError("mid-write".into()));
        }
        Ok(Document::new(self.sheets.borrow().clone()))
    }

    fn revision(&self, _: &Path) -> Option<u64> {
        *self.revision.borrow()
    }
}

/// Opens a viewer on a `SharedSource`, auto-reload wired like production.
pub(crate) fn viewer_on(source: &SharedSource) -> Viewer {
    viewer_on_with(source, Box::new(NullStore))
}

pub(crate) fn viewer_on_with(source: &SharedSource, store: Box<dyn CommentStore>) -> Viewer {
    Viewer::open(Box::new(source.clone()), store, Path::new("test.xlsx")).unwrap()
}

pub(crate) fn viewer(rows: usize, cols: usize) -> Viewer {
    viewer_with(rows, cols, Vec::new(), Box::new(NullStore))
}

pub(crate) fn viewer_with(
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

pub(crate) fn type_text(v: &mut Viewer, text: &str) {
    for c in text.chars() {
        if c == '\n' {
            v.apply(Event::Newline);
        } else {
            v.apply(Event::Insert(c));
        }
    }
}

pub(crate) fn thread(sheet: &str, row: u32, col: u32, resolved: bool) -> CommentThread {
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
