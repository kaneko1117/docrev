use thiserror::Error;

#[derive(Debug, Error)]
pub enum LoadError {
    #[error("{0}")]
    Open(String),
    #[error("{0}")]
    Sheet(String),
}

#[derive(Debug, Error)]
pub enum DocumentError {
    #[error("failed to load document: {0}")]
    Load(#[from] LoadError),
    #[error("sheet \"{name}\" not found. available sheets: {}", available.join(", "))]
    SheetNotFound {
        name: String,
        available: Vec<String>,
    },
    #[error("document has no sheets")]
    EmptyDocument,
}

#[derive(Debug, Error)]
#[error("{0}")]
pub struct FrontendError(pub String);

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("{0}")]
    Io(String),
    /// Unparsable JSON, or a stored anchor that is not a cell reference.
    #[error("{0}")]
    Corrupt(String),
    #[error("unsupported sidecar version {found} (supported: {supported})")]
    UnsupportedVersion { found: u32, supported: u32 },
    #[error("no thread with id {0}")]
    ThreadNotFound(String),
}

#[derive(Debug, Error)]
pub enum CommentError {
    #[error("invalid cell reference \"{0}\" (expected \"Sheet!B3\")")]
    BadReference(String),
    #[error(transparent)]
    Document(#[from] DocumentError),
    #[error(transparent)]
    Store(#[from] StoreError),
}
