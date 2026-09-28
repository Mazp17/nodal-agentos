//! Shared utilities: time, ids, paths, git and blocking I/O.
//!
//! `new_id`, `is_valid_id`, `check_id` and `clip_chars` moved to `nodal_domain::util`;
//! re-exported so current uses don't break.

pub mod git;
pub mod paths;

pub use nodal_domain::util::*;

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Runs `f` on a blocking thread (disk, git) without stalling the runtime.
pub async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| format!("Internal error: {e}"))?
}

/// Atomic write: temp file + rename (creates any missing folders).
pub fn write_atomic(path: &std::path::Path, contents: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&tmp, contents).map_err(|e| format!("Couldn't write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("Couldn't save {}: {e}", path.display()))
}
