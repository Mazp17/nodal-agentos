//! Paths: `~`, the canonical git root and Nodal's folder in the home directory.
//!
//! No tauri dependency: the app data folder (which needs a running Tauri app) is resolved by
//! the shell (`SRC/paths.rs`), not here.

use std::path::PathBuf;

use crate::git;

pub use crate::claude::fs::paths::claude_config_dir;

pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// `~` and `~/x` → home. Everything else stays as is.
pub fn expand_home(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), home()) {
        (Some(rest), Some(h)) => h.join(rest),
        _ if path == "~" => home().unwrap_or_default(),
        _ => PathBuf::from(path),
    }
}

/// `~/.nodal` (`~/.nodal-dev` in debug builds): the tasks' worktrees.
pub fn nodal_home() -> Result<PathBuf, String> {
    let name = if nodal_domain::DEV { ".nodal-dev" } else { ".nodal" };
    home().map(|h| h.join(name)).ok_or_else(|| "$HOME is not set.".to_string())
}

/// Absolute, existing folder (with `~` expanded).
fn existing_dir(path: &str) -> Result<PathBuf, String> {
    let dir = expand_home(path.trim());
    if !dir.is_absolute() {
        return Err(format!("The folder must be an absolute path: {path}"));
    }
    if !dir.is_dir() {
        return Err(format!("The folder doesn't exist: {path}"));
    }
    Ok(dir)
}

/// Canonical form (symlinks resolved) of an absolute, existing folder. Repo paths are stored
/// canonical too, so both can be compared with `Path::starts_with`.
pub fn canonical_dir(path: &str) -> Result<PathBuf, String> {
    let dir = existing_dir(path)?;
    dir.canonicalize().map_err(|e| format!("Couldn't resolve {}: {e}", dir.display()))
}

/// Canonical git root (symlinks resolved) of the repo containing `path`. `Ok(None)` if it's
/// not inside a git repo.
pub fn git_root_of(path: &str) -> Result<Option<PathBuf>, String> {
    let dir = existing_dir(path)?;
    let Some(root) = git::toplevel(&dir)? else { return Ok(None) };
    Ok(Some(root.canonicalize().map_err(|e| format!("Couldn't resolve {}: {e}", root.display()))?))
}

/// Canonical git root, or an error ready to display.
pub fn require_git_root(path: &str) -> Result<PathBuf, String> {
    git_root_of(path)?.ok_or_else(|| format!("{} is not inside a git repository.", path.trim()))
}

/// How many levels below the scanned folder a repo can be.
const SCAN_DEPTH: usize = 3;

/// Canonical git roots at or up to `SCAN_DEPTH` levels below `root`, sorted. Skips
/// `node_modules`, hidden folders (`.git` included), symlinks and anything inside a repo
/// already found. Only folders holding a `.git` (folder or file, for worktrees and
/// submodules) are handed to git, so a large tree doesn't spawn one process per folder.
pub fn git_repos_under(root: &str) -> Result<Vec<PathBuf>, String> {
    let root = canonical_dir(root)?;
    let mut found = Vec::new();
    let mut pending = vec![(root, 0)];
    while let Some((dir, depth)) = pending.pop() {
        if dir.join(".git").exists() && git_root_of(&dir.to_string_lossy())?.as_deref() == Some(dir.as_path()) {
            found.push(dir);
            continue;
        }
        if depth == SCAN_DEPTH {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || name == "node_modules" || !e.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            pending.push((e.path(), depth + 1));
        }
    }
    found.sort();
    Ok(found)
}

#[cfg(test)]
mod tests;
