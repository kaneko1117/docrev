use thiserror::Error;

#[derive(Debug, Error)]
#[error("{0}")]
pub struct LoadError(pub String);

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
