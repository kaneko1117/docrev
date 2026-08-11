use std::fs;
use std::io;
use std::path::Path;

/// Missing file is `Ok(None)` — a document without a sidecar is normal.
pub fn read_optional(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Sibling temp file + rename, so readers never observe a half-written file.
/// The temp name is unique per write so concurrent writers cannot clobber
/// each other's temp file (the last rename still wins; locking is #4's scope).
pub fn write_atomic(path: &Path, contents: &str) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let tmp = std::path::PathBuf::from(tmp);
    fs::write(&tmp, contents)?;
    let renamed = fs::rename(&tmp, path);
    if renamed.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    renamed
}
