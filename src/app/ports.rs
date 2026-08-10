use std::path::Path;

use crate::domain::document::Document;

use super::error::LoadError;

pub trait DocumentSource {
    fn load(&self, path: &Path) -> Result<Document, LoadError>;
}
