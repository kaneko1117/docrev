use std::path::Path;

use crate::domain::anchor::Anchor;
use crate::domain::comment::CommentThread;
use crate::domain::document::Document;

use super::error::{LoadError, StoreError};

pub trait DocumentSource {
    fn load(&self, path: &Path) -> Result<Document, LoadError>;
    /// Changes whenever the file changes; `None` disables auto-reload.
    fn revision(&self, _path: &Path) -> Option<u64> {
        None
    }
}

/// Implementations assign ids and timestamps.
pub trait CommentStore {
    fn load(&self) -> Result<Vec<CommentThread>, StoreError>;
    /// Changes whenever the store changes; `None` disables auto-reload.
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
    /// Continues the thread `existing` picks out of the stored ones, else starts one at `anchor`.
    /// The default picks and then writes as two steps; a store shared between processes must
    /// override it so both happen under one lock.
    fn comment_on(
        &mut self,
        anchor: Anchor,
        existing: &dyn Fn(&[CommentThread]) -> Option<String>,
        body: &str,
        author: &str,
    ) -> Result<CommentThread, StoreError> {
        match existing(&self.load()?) {
            Some(id) => self.add_reply(&id, body, author),
            None => self.add_thread(anchor, body, author),
        }
    }
}
