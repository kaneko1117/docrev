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
}
