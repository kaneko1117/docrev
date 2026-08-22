use std::path::{Path, PathBuf};

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::app::comments;
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

    fn lock_path(&self) -> PathBuf {
        let mut path = self.sidecar.as_os_str().to_owned();
        path.push(".lock");
        PathBuf::from(path)
    }

    /// Exclusive advisory lock preventing lost updates when the TUI and an
    /// agent CLI write concurrently.
    fn lock(&self) -> Result<fs::SidecarLock, StoreError> {
        fs::SidecarLock::acquire(&self.lock_path())
            .map_err(|e| StoreError(format!("cannot lock sidecar: {e}")))
    }

    fn read(&self) -> Result<SidecarFile, StoreError> {
        let Some(text) = fs::read_optional(&self.sidecar).map_err(|e| StoreError(e.to_string()))?
        else {
            return Ok(SidecarFile::default());
        };
        // our atomic writes never produce an empty file, so one is a shell
        // accident (`... > file.docrev.json` truncates before running);
        // treating it as empty recovers instead of failing forever
        if text.trim().is_empty() {
            return Ok(SidecarFile::default());
        }
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
    /// Modification time and size folded together: mtime alone can have
    /// one-second granularity, which would hide a quick second write.
    /// A missing sidecar is `Some(0)`, so an agent creating one registers
    /// as a change.
    fn revision(&self) -> Option<u64> {
        let metadata = match std::fs::metadata(&self.sidecar) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Some(0),
            Err(_) => return None,
        };
        let modified = metadata
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_millis() as u64;
        Some(modified.wrapping_mul(31).wrapping_add(metadata.len()))
    }

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
        let mut lock = self.lock()?;
        let _guard = lock
            .exclusive()
            .map_err(|e| StoreError(format!("cannot lock sidecar: {e}")))?;
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
        let mut lock = self.lock()?;
        let _guard = lock
            .exclusive()
            .map_err(|e| StoreError(format!("cannot lock sidecar: {e}")))?;
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
        // a reply reopens the thread: otherwise it is invisible to the viewer
        // (no marker) and to agents (which list unresolved threads)
        dto.resolved = false;
        let thread = dto.clone().into_domain()?;
        self.write(&file)?;
        Ok(thread)
    }

    fn resolve(&mut self, thread_id: &str) -> Result<(), StoreError> {
        let mut lock = self.lock()?;
        let _guard = lock
            .exclusive()
            .map_err(|e| StoreError(format!("cannot lock sidecar: {e}")))?;
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

/// CLI output: a single thread in the sidecar's thread shape.
pub fn thread_to_json(thread: &CommentThread) -> Result<String, StoreError> {
    serde_json::to_string_pretty(&ThreadDto::from_domain(thread))
        .map_err(|e| StoreError(e.to_string()))
}

/// CLI output for `list --json`: the sidecar shape, each thread augmented
/// with its derived `cell` content, plus the workbook's own comments in a
/// separate read-only array. Output-only — the sidecar file stores neither.
pub fn threads_with_context_to_json(list: &comments::ContextualList) -> Result<String, StoreError> {
    let items = &list.threads;
    #[derive(Serialize)]
    struct File {
        version: u32,
        comments: Vec<Entry>,
        /// The workbook's own, read-only — no id, never actionable.
        workbook_comments: Vec<WorkbookEntry>,
    }
    #[derive(Serialize)]
    struct WorkbookEntry {
        anchor: AnchorDto,
        author: String,
        body: String,
        resolved: bool,
        replies: Vec<WorkbookReplyDto>,
    }
    #[derive(Serialize)]
    struct WorkbookReplyDto {
        author: String,
        body: String,
    }
    #[derive(Serialize)]
    struct Entry {
        #[serde(flatten)]
        thread: ThreadDto,
        #[serde(skip_serializing_if = "Option::is_none")]
        cell: Option<CellDto>,
    }
    #[derive(Serialize)]
    struct CellDto {
        value: String,
        row: RowDto,
    }
    /// A JSON object in insertion order — `serde_json::Map` would sort keys
    /// alphabetically, putting AA1 before Z1 and breaking the promised
    /// column order for consumers that keep object order (Python dicts do).
    struct RowDto(Vec<(String, String)>);
    impl Serialize for RowDto {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeMap;
            let mut map = serializer.serialize_map(Some(self.0.len()))?;
            for (cell_ref, text) in &self.0 {
                map.serialize_entry(cell_ref, text)?;
            }
            map.end()
        }
    }
    let file = File {
        version: SCHEMA_VERSION,
        comments: items
            .iter()
            .map(|(thread, context)| Entry {
                thread: ThreadDto::from_domain(thread),
                cell: context.as_ref().map(|c| CellDto {
                    value: c.value.clone(),
                    row: RowDto(c.row.clone()),
                }),
            })
            .collect(),
        workbook_comments: list
            .workbook
            .iter()
            .map(|(sheet, comment)| WorkbookEntry {
                anchor: AnchorDto {
                    sheet: sheet.clone(),
                    cell: Anchor::cell("", comment.row as u32, comment.col as u32).cell_ref(),
                },
                author: comment.author.clone(),
                body: comment.body.clone(),
                resolved: comment.resolved,
                replies: comment
                    .replies
                    .iter()
                    .map(|r| WorkbookReplyDto {
                        author: r.author.clone(),
                        body: r.body.clone(),
                    })
                    .collect(),
            })
            .collect(),
    };
    serde_json::to_string_pretty(&file).map_err(|e| StoreError(e.to_string()))
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

    #[test]
    fn list_output_carries_derived_cell_content_but_only_when_present() {
        let thread = |id: &str| CommentThread {
            id: id.into(),
            anchor: Anchor::cell("IT-01", 1, 2),
            author: "user".into(),
            body: "fix this".into(),
            created_at: "2026-08-19T00:00:00Z".into(),
            resolved: false,
            replies: Vec::new(),
        };
        let items = vec![
            (
                thread("with"),
                Some(comments::CellContext {
                    value: "ロック表示".into(),
                    // Z before AA: column order, which alphabetical keys break
                    row: vec![("Z2".into(), "先".into()), ("AA2".into(), "後".into())],
                }),
            ),
            (thread("without"), None),
            (
                thread("empty-row"),
                Some(comments::CellContext {
                    value: String::new(),
                    row: Vec::new(),
                }),
            ),
        ];
        let list = comments::ContextualList {
            threads: items,
            workbook: vec![(
                "IT-01".to_string(),
                crate::domain::workbook_comment::WorkbookComment {
                    row: 0,
                    col: 0,
                    author: "\u{7530}\u{4e2d}".to_string(),
                    body: "\u{8981}\u{78ba}\u{8a8d}".to_string(),
                    resolved: true,
                    replies: vec![crate::domain::workbook_comment::WorkbookReply {
                        author: "\u{4f50}\u{85e4}".to_string(),
                        body: "\u{5bfe}\u{5fdc}\u{6e08}".to_string(),
                    }],
                },
            )],
        };
        let json = threads_with_context_to_json(&list).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["version"], 1, "still the sidecar shape");
        let first = &parsed["comments"][0];
        assert_eq!(first["anchor"]["cell"], "C2", "thread fields flatten");
        assert_eq!(first["cell"]["value"], "ロック表示");
        assert_eq!(first["cell"]["row"]["Z2"], "先");
        assert!(
            json.find("\"Z2\"").unwrap() < json.find("\"AA2\"").unwrap(),
            "row keys keep column order, not alphabetical order:\n{json}"
        );
        let second = &parsed["comments"][1];
        assert!(
            second.get("cell").is_none(),
            "degraded threads carry no cell key"
        );
        let third = &parsed["comments"][2];
        assert_eq!(
            third["cell"]["row"],
            serde_json::json!({}),
            "an empty row is an empty object, not a missing key"
        );
    }

    #[test]
    fn a_truncated_sidecar_reads_as_empty_instead_of_failing_forever() {
        let document = temp_document();
        let store = JsonCommentStore::for_document(&document);
        std::fs::write(&store.sidecar, "").unwrap();
        assert_eq!(store.load().unwrap(), Vec::new());
        cleanup(&document);
    }

    fn cleanup(document: &Path) {
        let store = JsonCommentStore::for_document(document);
        let _ = std::fs::remove_file(store.lock_path());
        let _ = std::fs::remove_file(store.sidecar);
    }

    #[test]
    fn concurrent_writers_do_not_lose_updates() {
        let document = temp_document();
        let handles: Vec<_> = (0..4)
            .map(|writer| {
                let doc = document.clone();
                std::thread::spawn(move || {
                    let mut store = JsonCommentStore::for_document(&doc);
                    for i in 0..5 {
                        store
                            .add_thread(Anchor::cell("s", writer, i), "x", "user")
                            .unwrap();
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        let threads = JsonCommentStore::for_document(&document).load().unwrap();
        assert_eq!(threads.len(), 20, "no update may be lost");
        cleanup(&document);
    }

    #[test]
    fn sidecar_path_appends_suffix() {
        let store = JsonCommentStore::for_document(Path::new("dir/budget.xlsx"));
        assert_eq!(store.sidecar, PathBuf::from("dir/budget.xlsx.docrev.json"));
    }

    #[test]
    fn revision_changes_when_the_sidecar_changes() {
        let document = temp_document();
        let mut store = JsonCommentStore::for_document(&document);
        assert_eq!(store.revision(), Some(0), "no sidecar yet");

        store
            .add_thread(Anchor::cell("s", 0, 0), "first", "user")
            .unwrap();
        let after_first = store.revision();
        assert!(after_first.is_some_and(|r| r != 0), "writing changed it");
        assert_eq!(store.revision(), after_first, "stable while untouched");

        store
            .add_thread(Anchor::cell("s", 1, 0), "second", "user")
            .unwrap();
        assert_ne!(store.revision(), after_first, "a second write is visible");
        cleanup(&document);
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
    fn a_reply_reopens_a_resolved_thread() {
        let document = temp_document();
        let mut store = JsonCommentStore::for_document(&document);
        let thread = store
            .add_thread(Anchor::cell("s", 0, 0), "is this right?", "user")
            .unwrap();
        store.resolve(&thread.id).unwrap();

        let replied = store.add_reply(&thread.id, "actually, no", "user").unwrap();
        assert!(!replied.resolved, "the answer revives the conversation");
        let reloaded = &store.load().unwrap()[0];
        assert!(!reloaded.resolved);
        assert_eq!(reloaded.replies.len(), 1);
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
