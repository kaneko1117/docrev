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
const SCHEMA_VERSION: u32 = 2;
/// Version 1 had cell anchors only; 2 added `{"kind": "line"}` and reads 1 unchanged.
const OLDEST_READABLE_VERSION: u32 = 1;

/// `<document>.docrev.json`.
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

    fn lock(&self) -> Result<fs::SidecarLock, StoreError> {
        fs::SidecarLock::acquire(&self.lock_path())
            .map_err(|e| StoreError::Io(format!("cannot lock sidecar: {e}")))
    }

    fn read(&self) -> Result<SidecarFile, StoreError> {
        let Some(text) =
            fs::read_optional(&self.sidecar).map_err(|e| StoreError::Io(e.to_string()))?
        else {
            return Ok(SidecarFile::default());
        };
        // an empty file is a shell accident (`> file` truncation), not a sidecar
        if text.trim().is_empty() {
            return Ok(SidecarFile::default());
        }
        let file: SidecarFile = serde_json::from_str(&text).map_err(|e| {
            StoreError::Corrupt(format!("invalid sidecar {}: {e}", self.sidecar.display()))
        })?;
        if !(OLDEST_READABLE_VERSION..=SCHEMA_VERSION).contains(&file.version) {
            return Err(StoreError::UnsupportedVersion {
                found: file.version,
                supported: SCHEMA_VERSION,
            });
        }
        // an older file is upgraded on its next write
        Ok(SidecarFile {
            version: SCHEMA_VERSION,
            ..file
        })
    }

    fn write(&self, file: &SidecarFile) -> Result<(), StoreError> {
        let json = serde_json::to_string_pretty(file).map_err(|e| StoreError::Io(e.to_string()))?;
        fs::write_atomic(&self.sidecar, &json).map_err(|e| StoreError::Io(e.to_string()))
    }
}

impl CommentStore for JsonCommentStore {
    fn revision(&self) -> Option<u64> {
        fs::revision(&self.sidecar)
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
            .map_err(|e| StoreError::Io(format!("cannot lock sidecar: {e}")))?;
        let mut file = self.read()?;
        let thread = push_thread(&mut file, anchor, body, author);
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
            .map_err(|e| StoreError::Io(format!("cannot lock sidecar: {e}")))?;
        let mut file = self.read()?;
        let thread = push_reply(&mut file, thread_id, body, author)?;
        self.write(&file)?;
        Ok(thread)
    }

    fn comment_on(
        &mut self,
        anchor: Anchor,
        existing: &dyn Fn(&[CommentThread]) -> Option<String>,
        body: &str,
        author: &str,
    ) -> Result<CommentThread, StoreError> {
        let mut lock = self.lock()?;
        let _guard = lock
            .exclusive()
            .map_err(|e| StoreError::Io(format!("cannot lock sidecar: {e}")))?;
        let mut file = self.read()?;
        let threads = file
            .comments
            .iter()
            .map(|dto| dto.clone().into_domain())
            .collect::<Result<Vec<_>, _>>()?;
        let thread = match existing(&threads) {
            Some(id) => push_reply(&mut file, &id, body, author)?,
            None => push_thread(&mut file, anchor, body, author),
        };
        self.write(&file)?;
        Ok(thread)
    }

    fn resolve(&mut self, thread_id: &str) -> Result<(), StoreError> {
        let mut lock = self.lock()?;
        let _guard = lock
            .exclusive()
            .map_err(|e| StoreError::Io(format!("cannot lock sidecar: {e}")))?;
        let mut file = self.read()?;
        let Some(dto) = file.comments.iter_mut().find(|t| t.id == thread_id) else {
            return Err(StoreError::ThreadNotFound(thread_id.to_string()));
        };
        dto.resolved = true;
        self.write(&file)
    }
}

fn push_thread(file: &mut SidecarFile, anchor: Anchor, body: &str, author: &str) -> CommentThread {
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
    thread
}

fn push_reply(
    file: &mut SidecarFile,
    thread_id: &str,
    body: &str,
    author: &str,
) -> Result<CommentThread, StoreError> {
    let Some(dto) = file.comments.iter_mut().find(|t| t.id == thread_id) else {
        return Err(StoreError::ThreadNotFound(thread_id.to_string()));
    };
    dto.replies.push(ReplyDto {
        id: Uuid::new_v4().to_string(),
        author: author.to_string(),
        body: body.to_string(),
        created_at: now(),
    });
    // a reply reopens the thread
    dto.resolved = false;
    dto.clone().into_domain()
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn thread_to_json(thread: &CommentThread) -> Result<String, StoreError> {
    serde_json::to_string_pretty(&ThreadDto::from_domain(thread))
        .map_err(|e| StoreError::Io(e.to_string()))
}

/// Output-only: `cell` and `workbook_comments` are never stored in the sidecar.
pub fn threads_with_context_to_json(list: &comments::ContextualList) -> Result<String, StoreError> {
    let items = &list.threads;
    #[derive(Serialize)]
    struct File {
        version: u32,
        comments: Vec<Entry>,
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
        /// Present only when true.
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        hidden: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        cell: Option<CellDto>,
        #[serde(skip_serializing_if = "Option::is_none")]
        line: Option<LineDto>,
    }
    #[derive(Serialize)]
    struct LineDto {
        text: String,
        /// 1-based line number -> text, in line order.
        context: NumberedDto,
    }
    struct NumberedDto(Vec<(u32, String)>);
    impl Serialize for NumberedDto {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeMap;
            let mut map = serializer.serialize_map(Some(self.0.len()))?;
            for (number, text) in &self.0 {
                map.serialize_entry(&number.to_string(), text)?;
            }
            map.end()
        }
    }
    #[derive(Serialize)]
    struct CellDto {
        value: String,
        /// Present only where a format produced the display.
        #[serde(skip_serializing_if = "Option::is_none")]
        raw: Option<RawDto>,
        row: RowDto,
    }
    /// A string for dates and times, a number for formatted numbers.
    #[derive(Serialize)]
    #[serde(untagged)]
    enum RawDto {
        Text(String),
        Integer(i64),
        Float(f64),
    }
    impl RawDto {
        fn from_domain(raw: &comments::RawValue) -> Self {
            match raw {
                comments::RawValue::DateTime(text) => RawDto::Text(text.clone()),
                // an integral value prints as one; `as i64` is exact below 2^63
                comments::RawValue::Number(n)
                    if n.fract() == 0.0 && n.abs() < 9_223_372_036_854_775_808.0 =>
                {
                    RawDto::Integer(*n as i64)
                }
                comments::RawValue::Number(n) => RawDto::Float(*n),
            }
        }
    }
    /// Insertion-ordered object; `serde_json::Map` would sort AA1 before Z1.
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
                hidden: context
                    .as_ref()
                    .is_some_and(comments::AnchorContext::hidden),
                cell: match context {
                    Some(comments::AnchorContext::Cell(c)) => Some(CellDto {
                        value: c.value.clone(),
                        raw: c.raw.as_ref().map(RawDto::from_domain),
                        row: RowDto(c.row.clone()),
                    }),
                    _ => None,
                },
                line: match context {
                    Some(comments::AnchorContext::Line(l)) if !l.hidden => Some(LineDto {
                        text: l.text.clone(),
                        context: NumberedDto(l.context.clone()),
                    }),
                    _ => None,
                },
            })
            .collect(),
        workbook_comments: list
            .workbook
            .iter()
            .map(|(sheet, comment)| WorkbookEntry {
                anchor: AnchorDto::from_domain(&Anchor::cell(
                    sheet.clone(),
                    comment.row as u32,
                    comment.col as u32,
                )),
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
    serde_json::to_string_pretty(&file).map_err(|e| StoreError::Io(e.to_string()))
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

/// A cell is `{sheet, cell}` with no `kind`; every other kind carries `kind` and its own keys.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AnchorDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sheet: Option<String>,
    /// A1 notation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cell: Option<String>,
    /// 1-based.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    line: Option<u32>,
}

impl AnchorDto {
    fn from_domain(anchor: &Anchor) -> Self {
        let mut dto = Self {
            kind: None,
            sheet: None,
            cell: None,
            line: None,
        };
        match anchor {
            Anchor::Cell { sheet, .. } => {
                dto.sheet = Some(sheet.clone());
                dto.cell = Some(anchor.position());
            }
            Anchor::Line { .. } => {
                dto.kind = Some("line".to_string());
                dto.line = anchor.line_number();
            }
        }
        dto
    }

    fn into_domain(self) -> Result<Anchor, StoreError> {
        match (self.kind.as_deref(), self.sheet, self.cell, self.line) {
            (None, Some(sheet), Some(cell), _) => {
                let (row, col) = Anchor::parse_cell_ref(&cell).ok_or_else(|| {
                    StoreError::Corrupt(format!("invalid cell reference \"{cell}\""))
                })?;
                Ok(Anchor::cell(sheet, row, col))
            }
            (None, _, _, _) => Err(StoreError::Corrupt(
                "anchor has neither a cell nor a kind".to_string(),
            )),
            (Some("line"), _, _, Some(line)) => Anchor::from_line_number(line)
                .ok_or_else(|| StoreError::Corrupt("line anchors start at 1".to_string())),
            (Some("line"), _, _, None) => {
                Err(StoreError::Corrupt("line anchor has no line".to_string()))
            }
            (Some(kind), _, _, _) => Err(StoreError::Corrupt(format!(
                "unknown anchor kind \"{kind}\""
            ))),
        }
    }
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
        Ok(CommentThread {
            id: self.id,
            anchor: self.anchor.into_domain()?,
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
            anchor: AnchorDto::from_domain(&thread.anchor),
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
    fn list_output_carries_a_line_object_for_shown_line_threads() {
        let thread = |id: &str, line: u32| CommentThread {
            id: id.into(),
            anchor: Anchor::line(line),
            author: "user".into(),
            body: "b".into(),
            created_at: "2026-09-11T00:00:00Z".into(),
            resolved: false,
            replies: Vec::new(),
        };
        let list = comments::ContextualList {
            threads: vec![
                (
                    thread("shown", 12),
                    Some(comments::AnchorContext::Line(comments::LineContext {
                        text: "brew install docrev".into(),
                        // numbers stay in line order, not string order
                        context: vec![(9, "## Install".into()), (11, "x".into()), (14, "y".into())],
                        hidden: false,
                    })),
                ),
                (
                    thread("gone", 40),
                    Some(comments::AnchorContext::Line(comments::LineContext {
                        text: String::new(),
                        context: Vec::new(),
                        hidden: true,
                    })),
                ),
                (thread("unread", 0), None),
            ],
            workbook: Vec::new(),
        };
        let json = threads_with_context_to_json(&list).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let shown = &parsed["comments"][0];
        assert_eq!(
            shown["anchor"],
            serde_json::json!({"kind": "line", "line": 13})
        );
        assert_eq!(shown["line"]["text"], "brew install docrev");
        let at = |key: &str| json.find(&format!("\"{key}\"")).unwrap();
        assert!(at("9") < at("11") && at("11") < at("14"), "insertion order");
        assert!(shown.get("cell").is_none() && shown.get("hidden").is_none());
        let gone = &parsed["comments"][1];
        assert_eq!(gone["hidden"], true);
        assert!(gone.get("line").is_none());
        let unread = &parsed["comments"][2];
        assert!(unread.get("line").is_none() && unread.get("hidden").is_none());
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
                Some(comments::AnchorContext::Cell(comments::CellContext {
                    value: "ロック表示".into(),
                    raw: Some(comments::RawValue::DateTime("2026-08-31 00:00:00".into())),
                    // Z before AA
                    row: vec![("Z2".into(), "先".into()), ("AA2".into(), "後".into())],
                    hidden: false,
                })),
            ),
            (thread("without"), None),
            (
                thread("empty-row"),
                Some(comments::AnchorContext::Cell(comments::CellContext {
                    value: String::new(),
                    raw: None,
                    row: Vec::new(),
                    hidden: true,
                })),
            ),
            (
                thread("scaled"),
                Some(comments::AnchorContext::Cell(comments::CellContext {
                    value: "1,234千円".into(),
                    raw: Some(comments::RawValue::Number(1_234_000.0)),
                    row: Vec::new(),
                    hidden: false,
                })),
            ),
            (
                thread("percent"),
                Some(comments::AnchorContext::Cell(comments::CellContext {
                    value: "15%".into(),
                    raw: Some(comments::RawValue::Number(0.15)),
                    row: Vec::new(),
                    hidden: false,
                })),
            ),
            (
                thread("huge"),
                Some(comments::AnchorContext::Cell(comments::CellContext {
                    value: "▲9,007,199,254,740,992".into(),
                    raw: Some(comments::RawValue::Number(-9_007_199_254_740_992.0)),
                    row: Vec::new(),
                    hidden: false,
                })),
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
        assert_eq!(parsed["version"], 2, "still the sidecar shape");
        let first = &parsed["comments"][0];
        assert_eq!(first["anchor"]["cell"], "C2", "thread fields flatten");
        assert_eq!(first["cell"]["value"], "ロック表示");
        assert_eq!(
            first["cell"]["raw"], "2026-08-31 00:00:00",
            "a date cell's machine-readable value rides along"
        );
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
        assert!(
            third["cell"].get("raw").is_none(),
            "plain cells carry no raw key"
        );
        let scaled = &parsed["comments"][3];
        assert_eq!(scaled["cell"]["value"], "1,234千円");
        assert_eq!(
            scaled["cell"]["raw"],
            serde_json::json!(1_234_000),
            "a formatted number's stored value is a JSON number, integral without a fraction"
        );
        assert!(json.contains("\"raw\": 1234000,"), "{json}");
        assert_eq!(
            parsed["comments"][4]["cell"]["raw"],
            serde_json::json!(0.15)
        );
        assert!(
            json.contains("\"raw\": -9007199254740992,"),
            "integral values past 2^53 still print as integers:\n{json}"
        );
        assert_eq!(third["hidden"], true);
        assert!(
            first.get("hidden").is_none() && second.get("hidden").is_none(),
            "hidden is emitted only when true"
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
        assert_eq!(reloaded[0].anchor.position(), "B3");
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
        assert!(matches!(err, StoreError::Corrupt(_)), "{err:?}");
        assert!(err.to_string().contains("invalid sidecar"), "{err}");
        cleanup(&document);
    }

    #[test]
    fn future_version_is_rejected() {
        let document = temp_document();
        let store = JsonCommentStore::for_document(&document);
        std::fs::write(&store.sidecar, r#"{"version": 3, "comments": []}"#).unwrap();
        let err = store.load().unwrap_err();
        assert!(
            matches!(
                err,
                StoreError::UnsupportedVersion {
                    found: 3,
                    supported: 2
                }
            ),
            "{err:?}"
        );
        assert!(err.to_string().contains("unsupported"), "{err}");
        cleanup(&document);
    }

    #[test]
    fn a_version_1_sidecar_loads_and_is_rewritten_as_version_2() {
        let document = temp_document();
        let mut store = JsonCommentStore::for_document(&document);
        std::fs::write(
            &store.sidecar,
            r#"{"version": 1, "comments": [{"id": "t1", "anchor": {"sheet": "s", "cell": "B3"},
                "author": "user", "body": "old", "created_at": "2026-01-01T00:00:00Z"}]}"#,
        )
        .unwrap();
        let threads = store.load().unwrap();
        assert_eq!(threads[0].anchor, Anchor::cell("s", 2, 1));

        store.resolve("t1").unwrap();
        let text = std::fs::read_to_string(&store.sidecar).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["version"], 2);
        assert_eq!(parsed["comments"][0]["anchor"]["cell"], "B3");
        assert!(
            parsed["comments"][0]["anchor"].get("kind").is_none(),
            "a cell anchor never carries a kind"
        );
        assert_eq!(store.load().unwrap()[0].anchor, Anchor::cell("s", 2, 1));
        cleanup(&document);
    }

    #[test]
    fn a_line_anchor_round_trips_as_a_one_based_kind_line_object() {
        let document = temp_document();
        let mut store = JsonCommentStore::for_document(&document);
        let thread = store
            .add_thread(Anchor::line(12), "wrong step", "user")
            .unwrap();
        assert_eq!(thread.anchor, Anchor::line(12));

        let text = std::fs::read_to_string(&store.sidecar).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["version"], 2);
        assert_eq!(
            parsed["comments"][0]["anchor"],
            serde_json::json!({"kind": "line", "line": 13})
        );
        assert_eq!(store.load().unwrap()[0].anchor, Anchor::line(12));
        assert!(thread_to_json(&thread).unwrap().contains(r#""line": 13"#));
        cleanup(&document);
    }

    #[test]
    fn anchors_docrev_cannot_read_are_corrupt() {
        let cases = [
            (r#"{"kind": "line", "line": 0}"#, "start at 1"),
            (r#"{"kind": "line"}"#, "no line"),
            (
                r#"{"kind": "cell", "sheet": "s", "cell": "B3"}"#,
                "unknown anchor kind",
            ),
            (
                r#"{"kind": "paragraph", "index": 1}"#,
                "unknown anchor kind",
            ),
            (r#"{"line": 13}"#, "neither a cell nor a kind"),
            (
                r#"{"sheet": "s", "cell": "nope"}"#,
                "invalid cell reference",
            ),
        ];
        for (anchor, expected) in cases {
            let document = temp_document();
            let store = JsonCommentStore::for_document(&document);
            std::fs::write(
                &store.sidecar,
                format!(
                    r#"{{"version": 2, "comments": [{{"id": "t", "anchor": {anchor},
                    "author": "u", "body": "b", "created_at": ""}}]}}"#
                ),
            )
            .unwrap();
            let err = store.load().unwrap_err();
            assert!(matches!(err, StoreError::Corrupt(_)), "{anchor}: {err:?}");
            assert!(err.to_string().contains(expected), "{anchor}: {err}");
            cleanup(&document);
        }
    }

    #[test]
    fn replying_to_a_missing_thread_is_thread_not_found() {
        let document = temp_document();
        let mut store = JsonCommentStore::for_document(&document);
        let err = store.add_reply("ghost", "x", "user").unwrap_err();
        assert!(matches!(err, StoreError::ThreadNotFound(_)), "{err:?}");
        assert!(err.to_string().contains("no thread with id ghost"), "{err}");
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
