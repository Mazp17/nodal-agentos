//! Input validation (projects, repos, tasks, executors, settings). Anything that touches
//! disk is blocking: it's called from `blocking`.
//!
//! Moved to `nodal_domain::board::validate`; re-exported here so current uses don't break.
//! `plan_file` and `root_path` stay here (they touch disk).

use std::path::{Path, PathBuf};

use crate::util::paths;

pub use nodal_domain::board::validate::*;

/// Plan file: `.md`, exists, is a file and stays inside `repo` after resolving symlinks and
/// `..` (canonicalize). `path` can be absolute or relative to the repo.
pub fn plan_file(repo: &Path, path: &str) -> Result<PathBuf, String> {
    let raw = path.trim();
    if raw.is_empty() {
        return Err("Pick a plan file.".into());
    }
    let repo = repo.canonicalize().map_err(|_| format!("The repository folder doesn't exist: {}", repo.display()))?;
    let p = Path::new(raw);
    let joined = if p.is_absolute() { p.to_path_buf() } else { repo.join(p) };
    let canon = joined.canonicalize().map_err(|_| format!("The plan file doesn't exist: {raw}"))?;
    if !canon.starts_with(&repo) {
        return Err(format!("The plan file must be inside the repository ({}).", repo.display()));
    }
    if canon.strip_prefix(&repo).is_ok_and(|rel| rel.components().any(|c| c.as_os_str() == ".git")) {
        return Err("The plan file can't be inside .git.".into());
    }
    let is_md = canon.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("md"));
    if !is_md {
        return Err(format!("The plan file must be a Markdown (.md) file: {raw}"));
    }
    let meta = std::fs::metadata(&canon).map_err(|e| format!("Couldn't read {raw}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("The plan path is not a file: {raw}"));
    }
    if meta.len() > MAX_PLAN_BYTES {
        return Err(format!("The plan file is too large (max {} KB).", MAX_PLAN_BYTES / 1024));
    }
    Ok(canon)
}

/// Project root folder: absolute, existing, stored canonical; empty → `None`.
pub fn root_path(s: Option<&str>) -> Result<Option<String>, String> {
    let Some(t) = s.map(str::trim).filter(|t| !t.is_empty()) else { return Ok(None) };
    Ok(Some(paths::canonical_dir(t)?.to_string_lossy().into_owned()))
}

#[cfg(test)]
mod tests;
