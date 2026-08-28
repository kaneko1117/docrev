use thiserror::Error;

/// Reading the document failed, spelled in the app's own terms — the
/// adapter translates format-specific errors into these variants.
#[derive(Debug, Error)]
pub enum LoadError {
    /// The file could not be opened as a workbook.
    #[error("{0}")]
    Open(String),
    /// The workbook opened but one of its sheets could not be read.
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

/// Sidecar access failed. The variant is the machine-readable part; the
/// payload keeps the human-facing message.
#[derive(Debug, Error)]
pub enum StoreError {
    /// Reading or writing the file failed.
    #[error("{0}")]
    Io(String),
    /// The sidecar exists but cannot be trusted: unparsable JSON or a
    /// stored anchor that is not a cell reference.
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
