use std::path::{Path, PathBuf};

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::app::error::StoreError;
use crate::app::ports::CommentStore;
use crate::domain::anchor::Anchor;
use crate::domain::comment::{CommentThread, Reply};
use crate::infra::fs;

pub const SIDECAR_SUFFIX: &str = ".docrev.json";
const SCHEMA_VERSION: u32 = 1;

/// Persists comments to `<document>.docrev.json`, never touching the document.
pub struct JsonCommentStore {
    sidecar: PathBuf,
}

impl JsonCommentStore {
    pub fn for_document(document: &Path) -> Self {
        let mut name = document.as_os_str().to_owned();
        name.push(SIDECAR_SUFFIX);
        Self {
            sidecar: PathBuf::from(name),
        }
    }

    fn read(&self) -> Result<SidecarFile, StoreError> {
        let Some(text) = fs::read_optional(&self.sidecar).map_err(|e| StoreError(e.to_string()))?
        else {
            return Ok(SidecarFile::default());
        };
        let file: SidecarFile = serde_json::from_str(&text)
            .map_err(|e| StoreError(format!("invalid sidecar {}: {e}", self.sidecar.display())))?;
        if file.version != SCHEMA_VERSION {
            return Err(StoreError(format!(
                "unsupported sidecar version {} (supported: {SCHEMA_VERSION})",
                file.version
            )));
        }
        Ok(file)
    }

    fn write(&self, file: &SidecarFile) -> Result<(), StoreError> {
        let json = serde_json::to_string_pretty(file).map_err(|e| StoreError(e.to_string()))?;
        fs::write_atomic(&self.sidecar, &json).map_err(|e| StoreError(e.to_string()))
    }
}

impl CommentStore for JsonCommentStore {
    fn load(&self) -> Result<Vec<CommentThread>, StoreError> {
        self.read()?
            .comments
            .into_iter()
            .map(ThreadDto::into_domain)
            .collect()
    }

    fn add_thread(
        &mut self,
        anchor: Anchor,
        body: &str,
        author: &str,
    ) -> Result<CommentThread, StoreError> {
        let mut file = self.read()?;
        let thread = CommentThread {
            id: Uuid::new_v4().to_string(),
            anchor,
            author: author.to_string(),
            body: body.to_string(),
            created_at: now(),
            resolved: false,
            replies: Vec::new(),
        };
        file.comments.push(ThreadDto::from_domain(&thread));
        self.write(&file)?;
        Ok(thread)
    }

    fn add_reply(
        &mut self,
        thread_id: &str,
        body: &str,
        author: &str,
    ) -> Result<CommentThread, StoreError> {
        let mut file = self.read()?;
        let Some(dto) = file.comments.iter_mut().find(|t| t.id == thread_id) else {
            return Err(StoreError(format!("no thread with id {thread_id}")));
        };
        dto.replies.push(ReplyDto {
            id: Uuid::new_v4().to_string(),
            author: author.to_string(),
            body: body.to_string(),
            created_at: now(),
        });
        let thread = dto.clone().into_domain()?;
        self.write(&file)?;
        Ok(thread)
    }

    fn resolve(&mut self, thread_id: &str) -> Result<(), StoreError> {
        let mut file = self.read()?;
        let Some(dto) = file.comments.iter_mut().find(|t| t.id == thread_id) else {
            return Err(StoreError(format!("no thread with id {thread_id}")));
        };
        dto.resolved = true;
        self.write(&file)
    }
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[derive(Debug, Serialize, Deserialize)]
struct SidecarFile {
    version: u32,
    #[serde(default)]
    comments: Vec<ThreadDto>,
}

impl Default for SidecarFile {
    fn default() -> Self {
        Self {
            version: SCHEMA_VERSION,
            comments: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ThreadDto {
    id: String,
    anchor: AnchorDto,
    author: String,
    body: String,
    created_at: String,
    #[serde(default)]
    resolved: bool,
    #[serde(default)]
    replies: Vec<ReplyDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AnchorDto {
    sheet: String,
    /// A1 notation — the sidecar is an edge (see CLAUDE.md).
    cell: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReplyDto {
    id: String,
    author: String,
    body: String,
    created_at: String,
}

impl ThreadDto {
    fn into_domain(self) -> Result<CommentThread, StoreError> {
        let (row, col) = Anchor::parse_cell_ref(&self.anchor.cell).ok_or_else(|| {
            StoreError(format!("invalid cell reference \"{}\"", self.anchor.cell))
        })?;
        Ok(CommentThread {
            id: self.id,
            anchor: Anchor::cell(self.anchor.sheet, row, col),
            author: self.author,
            body: self.body,
            created_at: self.created_at,
            resolved: self.resolved,
            replies: self
                .replies
                .into_iter()
                .map(|r| Reply {
                    id: r.id,
                    author: r.author,
                    body: r.body,
                    created_at: r.created_at,
                })
                .collect(),
        })
    }

    fn from_domain(thread: &CommentThread) -> Self {
        Self {
            id: thread.id.clone(),
            anchor: AnchorDto {
                sheet: thread.anchor.sheet().to_string(),
                cell: thread.anchor.cell_ref(),
            },
            author: thread.author.clone(),
            body: thread.body.clone(),
            created_at: thread.created_at.clone(),
            resolved: thread.resolved,
            replies: thread
                .replies
                .iter()
                .map(|r| ReplyDto {
                    id: r.id.clone(),
                    author: r.author.clone(),
                    body: r.body.clone(),
                    created_at: r.created_at.clone(),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_document() -> PathBuf {
        std::env::temp_dir().join(format!("docrev-test-{}.xlsx", Uuid::new_v4()))
    }

    fn cleanup(document: &Path) {
        let _ = std::fs::remove_file(JsonCommentStore::for_document(document).sidecar);
    }

    #[test]
    fn sidecar_path_appends_suffix() {
        let store = JsonCommentStore::for_document(Path::new("dir/budget.xlsx"));
        assert_eq!(store.sidecar, PathBuf::from("dir/budget.xlsx.docrev.json"));
    }

    #[test]
    fn missing_sidecar_is_empty() {
        let store = JsonCommentStore::for_document(&temp_document());
        assert_eq!(store.load().unwrap(), vec![]);
    }

    #[test]
    fn add_thread_and_reply_survive_reload() {
        let document = temp_document();
        let mut store = JsonCommentStore::for_document(&document);
        let thread = store
            .add_thread(Anchor::cell("売上", 2, 1), "単価が古い?", "user")
            .unwrap();
        store
            .add_reply(&thread.id, "確認しました", "claude")
            .unwrap();

        let reloaded = JsonCommentStore::for_document(&document).load().unwrap();
        assert_eq!(reloaded.len(), 1);
        assert_eq!(reloaded[0].anchor.cell_ref(), "B3");
        assert_eq!(reloaded[0].replies.len(), 1);
        assert_eq!(reloaded[0].replies[0].author, "claude");
        cleanup(&document);
    }

    #[test]
    fn resolve_marks_thread() {
        let document = temp_document();
        let mut store = JsonCommentStore::for_document(&document);
        let thread = store
            .add_thread(Anchor::cell("s", 0, 0), "x", "user")
            .unwrap();
        store.resolve(&thread.id).unwrap();
        assert!(store.load().unwrap()[0].resolved);
        cleanup(&document);
    }

    #[test]
    fn corrupt_sidecar_is_a_clear_error() {
        let document = temp_document();
        let store = JsonCommentStore::for_document(&document);
        std::fs::write(&store.sidecar, "{not json").unwrap();
        let err = store.load().unwrap_err();
        assert!(err.to_string().contains("invalid sidecar"), "{err}");
        cleanup(&document);
    }

    #[test]
    fn future_version_is_rejected() {
        let document = temp_document();
        let store = JsonCommentStore::for_document(&document);
        std::fs::write(&store.sidecar, r#"{"version": 2, "comments": []}"#).unwrap();
        let err = store.load().unwrap_err();
        assert!(err.to_string().contains("unsupported"), "{err}");
        cleanup(&document);
    }

    #[test]
    fn unknown_thread_id_is_an_error() {
        let document = temp_document();
        let mut store = JsonCommentStore::for_document(&document);
        assert!(store.add_reply("nope", "x", "claude").is_err());
        assert!(store.resolve("nope").is_err());
        cleanup(&document);
    }
}
