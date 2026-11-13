//! Lists one directory of a repo for the UI: validated against the repo root (trust
//! boundary: `rel` comes from the frontend), `.git` and git-ignored entries filtered out.

use std::path::{Component, Path, PathBuf};

use nodal_domain::model::{DirEntry, RepoFile, RepoWorktree, DIR_ENTRIES_MAX};

use super::{ok, run};

/// Names handed to one `git check-ignore` call (keeps argv small).
const CHECK_CHUNK: usize = 200;
/// Most bytes of a file the viewer gets.
const FILE_MAX: u64 = 1024 * 1024;
/// Most bytes `write_file` accepts.
const WRITE_MAX: u64 = 2 * 1024 * 1024;
/// Leading bytes sniffed for a NUL to call a file binary.
const BINARY_SNIFF: usize = 8 * 1024;

/// `rel` must be relative and made only of normal components (no `..`).
fn check_rel(rel: &str) -> Result<&Path, String> {
    let rel_path = Path::new(rel);
    if rel_path.is_absolute()
        || !rel_path
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err("Path must be relative to the repo and can't contain `..`.".into());
    }
    Ok(rel_path)
}

/// `rel` (from the frontend) resolved under `repo`: relative, no `..`, and the canonical result
/// (symlinks followed) still inside the canonical repo root.
fn resolve(repo: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel_path = check_rel(rel)?;
    let root = dunce::canonicalize(repo).map_err(|e| format!("Repo folder unavailable: {e}"))?;
    let path = dunce::canonicalize(root.join(rel_path))
        .map_err(|e| format!("Can't read `{rel}`: {e}"))?;
    // Symlinks are resolved above, so this also rejects links pointing outside the repo.
    if !path.starts_with(&root) {
        return Err("Path resolves outside the repo.".into());
    }
    Ok(path)
}

/// Reads a repo file for the read-only viewer: first `FILE_MAX` bytes, cut at a UTF-8 boundary.
/// Binary (NUL or invalid UTF-8 in the first 8 KB, or an invalid cut) comes back empty.
pub fn read_file(repo: &Path, rel: &str) -> Result<RepoFile, String> {
    use std::io::Read;
    let path = resolve(repo, rel)?;
    // Windows can't open a folder as a file, so `File::open` would fail with a vaguer error.
    if path.is_dir() {
        return Err(format!("`{rel}` is not a regular file."));
    }
    let file = std::fs::File::open(&path).map_err(|e| format!("Can't read `{rel}`: {e}"))?;
    let meta = file.metadata().map_err(|e| format!("Can't read `{rel}`: {e}"))?;
    if !meta.is_file() {
        return Err(format!("`{rel}` is not a regular file."));
    }
    let size = meta.len();
    let mut buf = Vec::new();
    file.take(FILE_MAX).read_to_end(&mut buf).map_err(|e| format!("Can't read `{rel}`: {e}"))?;
    let truncated = size > FILE_MAX;
    let version = version_of(&meta, &buf);
    let mk = |content: String| RepoFile { version: version.clone(), content, binary: false, truncated, size };
    let head = &buf[..buf.len().min(BINARY_SNIFF)];
    if head.contains(&0) {
        return Ok(RepoFile { binary: true, ..mk(String::new()) });
    }
    match String::from_utf8(buf) {
        Ok(s) => Ok(mk(s)),
        // A multi-byte char cut by the 1 MB limit: keep the valid prefix.
        Err(e) if truncated && e.utf8_error().error_len().is_none() => {
            let valid = e.utf8_error().valid_up_to();
            let mut bytes = e.into_bytes();
            bytes.truncate(valid);
            Ok(mk(String::from_utf8(bytes).unwrap_or_default()))
        }
        Err(_) => Ok(RepoFile { binary: true, ..mk(String::new()) }),
    }
}

/// Opaque change token: modification time (ns), size and an FNV-1a hash of the (capped)
/// bytes, so a coarse mtime can't hide a same-size edit.
fn version_of(meta: &std::fs::Metadata, bytes: &[u8]) -> String {
    let nanos = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    let hash = bytes.iter().fold(0xcbf29ce484222325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3));
    format!("{nanos}-{}-{hash:x}", meta.len())
}

/// `version_of` for the file as it is on disk right now.
fn version_at(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    let meta = file.metadata()?;
    let mut buf = Vec::new();
    file.take(FILE_MAX).read_to_end(&mut buf)?;
    Ok(version_of(&meta, &buf))
}

/// Saves `content` over an existing, fully loadable text file. Fails with "File changed on
/// disk" if its version isn't `version` (unless `force`). Atomic: a temp file next to the
/// target (same permissions) is fsynced and renamed over it; bytes are written as given.
/// The version is re-checked after the temp file is fsynced, right before the rename.
/// shortcut: that last check and the rename still aren't atomic, so an edit landing in the
/// microseconds between them is overwritten; upgrade with file locking if that ever matters.
pub fn write_file(repo: &Path, rel: &str, content: &str, version: &str, force: bool) -> Result<RepoFile, String> {
    use std::io::Write;
    if content.len() as u64 > WRITE_MAX {
        return Err(format!("Content is larger than {} MB.", WRITE_MAX / (1024 * 1024)));
    }
    let current = read_file(repo, rel)?; // validates the path; errors if not a regular file
    if current.binary || current.truncated {
        return Err(format!("`{rel}` is binary or larger than 1 MB and can't be edited."));
    }
    if !force && current.version != version {
        return Err(format!("File changed on disk: `{rel}` was modified after it was opened."));
    }
    let path = resolve(repo, rel)?;
    let meta = std::fs::metadata(&path).map_err(|e| format!("Can't write `{rel}`: {e}"))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.nodal-{}.tmp", std::process::id()));
    let write = || -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(content.as_bytes())?;
        f.set_permissions(meta.permissions())?;
        f.sync_all()
    };
    if let Err(e) = write() {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("Couldn't save `{rel}`: {e}"));
    }
    if !force && !matches!(version_at(&path), Ok(v) if v == version) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("File changed on disk: `{rel}` was modified while saving."));
    }
    if let Err(e) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("Couldn't save `{rel}`: {e}"));
    }
    read_file(repo, rel)
}

/// The repo's worktrees, main first (`git worktree list --porcelain`), without prunable ones
/// or ones whose folder is gone.
pub fn list_worktrees(repo: &Path) -> Result<Vec<RepoWorktree>, String> {
    let out = ok(repo, &["worktree", "list", "--porcelain"])?;
    let mut list = Vec::new();
    for block in out.split("\n\n") {
        let mut path = None;
        let mut branch = None;
        let mut skip = false;
        for line in block.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                path = Some(super::native_path(p).to_string_lossy().into_owned());
            } else if let Some(b) = line.strip_prefix("branch ") {
                branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_string());
            } else if line.starts_with("prunable") || line == "bare" {
                skip = true;
            }
        }
        if let Some(path) = path.filter(|p| !skip && Path::new(p).is_dir()) {
            list.push(RepoWorktree { is_main: list.is_empty(), path, branch });
        }
    }
    Ok(list)
}

/// Throws away one file's uncommitted changes (index and working tree): restores it from HEAD
/// if HEAD has it, otherwise unstages and deletes it; a staged rename restores the old path
/// and removes the new one. Only files `git status` reports as changed are touched, so
/// ignored or clean files are never deleted. `rel` may not exist on disk (a deletion).
pub fn revert_file(root: &Path, rel: &str) -> Result<(), String> {
    let rel_path = check_rel(rel)?;
    if rel_path.components().any(|c| c.as_os_str() == ".git") {
        return Err("Can't revert files inside `.git`.".into());
    }
    let norm: PathBuf = rel_path.components().filter(|c| matches!(c, Component::Normal(_))).collect();
    if norm.as_os_str().is_empty() {
        return Err("Pick a file to revert.".into());
    }
    let root = dunce::canonicalize(root).map_err(|e| format!("Repo folder unavailable: {e}"))?;
    let full = root.join(&norm);
    // The file itself may be gone: validate its nearest existing ancestor instead.
    let near = full.ancestors().skip(1).find(|a| a.exists()).unwrap_or(&root);
    let near = dunce::canonicalize(near).map_err(|e| format!("Can't revert `{rel}`: {e}"))?;
    if !near.starts_with(&root) {
        return Err("Path resolves outside the repo.".into());
    }
    let norm = norm.to_string_lossy().replace('\\', "/");
    let status = ok(&root, &["status", "--porcelain=v1", "-z", "--untracked-files=all"])?;
    let mut entries = status.split('\0');
    let mut found: Option<Option<String>> = None; // Some(Some(old)) for a rename
    while let Some(e) = entries.next() {
        let (Some(xy), Some(path)) = (e.get(..2), e.get(3..)) else { continue };
        let old = if xy.contains('R') { entries.next().map(str::to_string) } else { None };
        if path == norm {
            found = Some(old);
        }
    }
    let Some(old) = found else {
        return Err(format!("`{rel}` has no uncommitted changes."));
    };
    if let Some(old) = old {
        check_rel(&old)?;
        ok(&root, &["restore", "--source=HEAD", "--staged", "--worktree", "--", &old])?;
        unstage_and_delete(&root, &full, &norm)?;
        return Ok(());
    }
    let in_head = run(&root, &["cat-file", "-e", &format!("HEAD:{norm}")]).is_ok_and(|o| o.ok);
    if in_head {
        ok(&root, &["restore", "--source=HEAD", "--staged", "--worktree", "--", &norm])?;
    } else {
        unstage_and_delete(&root, &full, &norm)?;
    }
    Ok(())
}

fn unstage_and_delete(root: &Path, full: &Path, norm: &str) -> Result<(), String> {
    ok(root, &["rm", "--cached", "-q", "-f", "--ignore-unmatch", "--", norm])?;
    match std::fs::symlink_metadata(full) {
        Ok(m) if m.is_dir() => Err(format!("`{norm}` is a directory.")),
        Ok(_) => std::fs::remove_file(full).map_err(|e| format!("Couldn't delete `{norm}`: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("Couldn't delete `{norm}`: {e}")),
    }
}

/// `git ls-files` (tracked + untracked, ignored left out), capped at `REPO_FILES_MAX`.
pub fn list_files(repo: &Path) -> Result<Vec<String>, String> {
    let out = ok(repo, &["ls-files", "--cached", "--others", "--exclude-standard", "-z"])?;
    let mut files: Vec<String> = out.split('\0').filter(|s| !s.is_empty() && !s.ends_with('/')).map(str::to_string).collect();
    files.dedup();
    files.truncate(nodal_domain::model::REPO_FILES_MAX);
    Ok(files)
}

pub fn list_dir(repo: &Path, rel: &str) -> Result<Vec<DirEntry>, String> {
    let dir = resolve(repo, rel)?;
    let mut all: Vec<DirEntry> = std::fs::read_dir(&dir)
        .map_err(|e| format!("Can't read `{rel}`: {e}"))?
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            (name != ".git").then(|| DirEntry { is_dir: e.path().is_dir(), name })
        })
        .collect();
    all.sort_by_cached_key(|e| (!e.is_dir, e.name.to_lowercase(), e.name.clone()));

    let mut out = Vec::new();
    for chunk in all.chunks(CHECK_CHUNK) {
        let ignored = ignored_names(&dir, chunk);
        out.extend(chunk.iter().filter(|e| !ignored.contains(&e.name)).cloned());
        if out.len() >= DIR_ENTRIES_MAX {
            out.truncate(DIR_ENTRIES_MAX);
            break;
        }
    }
    Ok(out)
}

/// Entries of `dir` that git ignores. Directories get a trailing `/` so dir-only patterns match.
/// shortcut: a failed `check-ignore` (not a git repo) or names git quotes (newline, quote, backslash) shows the entries
/// unfiltered; upgrade to `--stdin` if literal matching ever matters.
fn ignored_names(dir: &Path, entries: &[DirEntry]) -> Vec<String> {
    let paths: Vec<String> = entries
        .iter()
        .map(|e| if e.is_dir { format!("{}/", e.name) } else { e.name.clone() })
        .collect();
    let mut args = vec!["-c", "core.quotePath=false", "check-ignore", "--"];
    args.extend(paths.iter().map(String::as_str));
    // Exit 1 just means "nothing ignored", so only stdout is read.
    let Ok(out) = run(dir, &args) else { return Vec::new() };
    out.stdout
        .lines()
        .filter(|s| !s.is_empty())
        .map(|s| s.trim_end_matches('/').to_string())
        .collect()
}

#[cfg(test)]
mod tests;
