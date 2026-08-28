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

/// Advisory lock for sidecar read-modify-write cycles, backed by a
/// `<sidecar>.lock` file. Released on drop.
pub struct SidecarLock {
    lock: fd_lock::RwLock<fs::File>,
}

impl SidecarLock {
    pub fn acquire(lock_path: &Path) -> io::Result<Self> {
        // read+write, not append: Windows' LockFileEx denies handles that
        // carry only append permission
        let file = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(lock_path)?;
        Ok(Self {
            lock: fd_lock::RwLock::new(file),
        })
    }

    /// Blocks until the exclusive lock is granted.
    pub fn exclusive(&mut self) -> io::Result<fd_lock::RwLockWriteGuard<'_, fs::File>> {
        self.lock.write()
    }
}

/// Sibling temp file + rename, so readers never observe a half-written file.
/// The temp name is unique per write so concurrent writers cannot clobber
/// each other's temp file (the last rename still wins; writers needing
/// exclusion hold the sidecar lock).
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
