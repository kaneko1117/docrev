use thiserror::Error;

#[derive(Debug, Error)]
pub enum LoadError {
    #[error("{0}")]
    Open(String),
    #[error("unsupported file type {extension:?} (supported: {})", supported.join(", "))]
    Unsupported {
        extension: String,
        supported: Vec<String>,
    },
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
    #[error("document has no sheets: it is a text file")]
    NotAWorkbook,
    #[error("--formulas does not apply to a text file")]
    NoFormulas,
}

#[derive(Debug, Error)]
#[error("{0}")]
pub struct FrontendError(pub String);

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("{0}")]
    Io(String),
    /// Unparsable JSON, or a stored anchor docrev cannot read.
    #[error("{0}")]
    Corrupt(String),
    #[error("unsupported sidecar version {found} (supported: 1 to {supported})")]
    UnsupportedVersion { found: u32, supported: u32 },
    #[error("no thread with id {0}")]
    ThreadNotFound(String),
}

#[derive(Debug, Error)]
pub enum CommentError {
    #[error("invalid cell reference \"{0}\" (expected \"Sheet!B3\")")]
    BadReference(String),
    #[error("line numbers start at 1")]
    BadLine,
    #[error("specify the target with --cell or --line")]
    MissingTarget,
    #[error("document has no cells: it is a text file (use --line)")]
    NoCells,
    #[error("document has no lines: it is a workbook (use --cell)")]
    NoLines,
    #[error("line {line} is beyond the end of the file ({len} lines)")]
    LineOutOfRange { line: u32, len: usize },
    #[error("--sheet does not apply to a text file")]
    SheetFilterOnText,
    #[error(transparent)]
    Document(#[from] DocumentError),
    #[error(transparent)]
    Store(#[from] StoreError),
}
