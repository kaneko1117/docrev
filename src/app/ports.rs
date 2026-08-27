use std::path::Path;

use crate::domain::anchor::Anchor;
use crate::domain::comment::CommentThread;
use crate::domain::document::Document;

use super::error::{LoadError, StoreError};

pub trait DocumentSource {
    fn load(&self, path: &Path) -> Result<Document, LoadError>;
    /// A cheap token that changes whenever the file behind `path` changes,
    /// so the viewer can notice an outside edit without re-parsing.
    /// `None` when the source cannot report one — auto-reload then stays off.
    fn revision(&self, _path: &Path) -> Option<u64> {
        None
    }
}

/// Comment persistence. Implementations assign ids and timestamps.
pub trait CommentStore {
    fn load(&self) -> Result<Vec<CommentThread>, StoreError>;
    /// A cheap token that changes whenever the backing store changes, so the
    /// viewer can notice an agent's edits without re-reading the file.
    /// `None` when the store cannot report one — auto-reload then stays off.
    fn revision(&self) -> Option<u64> {
        None
    }
    fn add_thread(
        &mut self,
        anchor: Anchor,
        body: &str,
        author: &str,
    ) -> Result<CommentThread, StoreError>;
    fn add_reply(
        &mut self,
        thread_id: &str,
        body: &str,
        author: &str,
    ) -> Result<CommentThread, StoreError>;
    fn resolve(&mut self, thread_id: &str) -> Result<(), StoreError>;
}
