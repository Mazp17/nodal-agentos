//! Files Nodal owns (plans, stopped patches) and plan files inside repos. Blocking.
//!
//! Backs the `PlanFiles` port. `is_file`/`read_plan`/`write_atomic`/`remove_*`/`rename` are
//! generic file operations; `validate_plan_file` is `work::validate::plan_file` (checks a
//! plan path stays inside its repo); `write_plan` only writes when the content changed
//! (`providers::plan::write_plan`); `plan_path` computes where an imported task's plan
//! lives (`<data_dir>/tasks/<id>/plan.md`, `providers::plan::plan_path`).

use std::io::Read;
use std::path::{Path, PathBuf};

use nodal_domain::board::validate::MAX_PLAN_BYTES;
use nodal_domain::sources::plan_render::{PLANS_DIR, PLAN_FILE};

pub fn is_file(path: &Path) -> bool {
    path.is_file()
}

/// Reads a plan file, rejecting it if it's over `MAX_PLAN_BYTES`.
pub fn read_plan(path: &Path) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("Couldn't read the plan: {e}"))?;
    let mut bytes = Vec::new();
    file.take(MAX_PLAN_BYTES + 1).read_to_end(&mut bytes).map_err(|e| format!("Couldn't read the plan: {e}"))?;
    if bytes.len() as u64 > MAX_PLAN_BYTES {
        return Err(format!("The plan is too large (max {} KB).", MAX_PLAN_BYTES / 1024));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

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

/// Atomic write: temp file + rename (creates any missing folders).
pub fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&tmp, contents).map_err(|e| format!("Couldn't write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("Couldn't save {}: {e}", path.display()))
}

/// `<data_dir>/tasks/<id>/plan.md` (same path as the plan of local tasks).
pub fn plan_path(data_dir: &Path, task_id: &str) -> PathBuf {
    data_dir.join(PLANS_DIR).join(task_id).join(PLAN_FILE)
}

/// Writes the plan if it changed. Returns `true` if it wrote.
pub fn write_plan(path: &Path, content: &str) -> std::io::Result<bool> {
    if std::fs::read_to_string(path).is_ok_and(|old| old == content) {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, content)?;
    Ok(true)
}

pub fn remove_dir_all(dir: &Path) -> std::io::Result<()> {
    std::fs::remove_dir_all(dir)
}

pub fn remove_file(path: &Path) -> std::io::Result<()> {
    std::fs::remove_file(path)
}

pub fn rename(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::rename(from, to)
}

#[cfg(test)]
mod tests;
