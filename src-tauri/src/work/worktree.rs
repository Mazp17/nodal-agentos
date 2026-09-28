//! Task worktrees: `~/.nodal/worktrees/<repo>/<task-slug>` with the branch
//! `nodal/<task-slug>`, created from the repo's current branch before the first run and
//! reused by the following ones (handoffs included). All blocking.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::domain::WorktreeRef;
use crate::util::git;

pub const BRANCH_PREFIX: &str = "nodal/";
const SLUG_MAX: usize = 48;

/// Lowercase ASCII, digits and `-` (not repeated, not at the edges).
pub fn slugify(s: &str, max: usize) -> String {
    let mut out = String::new();
    for c in s.chars() {
        let c = match c {
            'á' | 'à' | 'ä' | 'â' | 'Á' | 'À' | 'Ä' | 'Â' => 'a',
            'é' | 'è' | 'ë' | 'ê' | 'É' | 'È' | 'Ë' | 'Ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' | 'Í' | 'Ì' | 'Ï' | 'Î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' | 'Ó' | 'Ò' | 'Ö' | 'Ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' | 'Ú' | 'Ù' | 'Ü' | 'Û' => 'u',
            'ñ' | 'Ñ' => 'n',
            c => c,
        };
        if c.is_ascii_alphanumeric() {
            if out.len() >= max {
                break;
            }
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            if out.len() + 1 >= max {
                break;
            }
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// `pay-1-new-logo-in-the-header`: the task key (unique in the app) plus the title.
pub fn task_slug(project_key: &str, number: i64, title: &str) -> String {
    let key = slugify(&format!("{project_key}-{number}"), 20);
    let rest = slugify(title, SLUG_MAX.saturating_sub(key.len() + 1));
    if rest.is_empty() {
        key
    } else {
        format!("{key}-{rest}")
    }
}

pub fn branch_for(slug: &str) -> String {
    format!("{BRANCH_PREFIX}{slug}")
}

/// Worktree folder: `<root>/<repo>/<slug>` (`root` = `~/.nodal/worktrees`).
pub fn dir_for(root: &Path, repo_name: &str, slug: &str) -> PathBuf {
    let repo = slugify(repo_name, 60);
    root.join(if repo.is_empty() { "repo".to_string() } else { repo }).join(slug)
}

/// The repo's current branch (or the commit if on a detached HEAD): the worktree's base.
pub fn current_base(repo: &Path) -> Result<String, String> {
    let out = git::run(repo, &["symbolic-ref", "--short", "-q", "HEAD"])?;
    let branch = out.stdout.trim();
    if out.ok && !branch.is_empty() {
        return Ok(branch.to_string());
    }
    Ok(git::ok(repo, &["rev-parse", "HEAD"])?.trim().to_string())
}

pub(crate) fn branch_exists(repo: &Path, branch: &str) -> Result<bool, String> {
    Ok(git::run(repo, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")])?.ok)
}

/// Is `dir` a live worktree (its toplevel is itself)?
pub(crate) fn is_live_worktree(dir: &Path) -> bool {
    dir.is_dir()
        && git::toplevel(dir)
            .ok()
            .flatten()
            .and_then(|t| t.canonicalize().ok())
            .zip(dir.canonicalize().ok())
            .is_some_and(|(a, b)| a == b)
}

/// Creates (or reuses) the task's worktree. `existing`: the one it already has stored.
pub fn ensure(repo: &Path, dir: &Path, branch: &str, existing: Option<&WorktreeRef>) -> Result<WorktreeRef, String> {
    if let Some(wt) = existing {
        let p = Path::new(&wt.path);
        if is_live_worktree(p) {
            return Ok(wt.clone());
        }
    }
    let (dir, branch) = match existing {
        Some(wt) => (PathBuf::from(&wt.path), wt.branch.clone()),
        None => (dir.to_path_buf(), branch.to_string()),
    };
    let base = match existing {
        Some(wt) => wt.base.clone(),
        None => current_base(repo)?,
    };
    // Records of worktrees deleted by hand.
    let _ = git::run(repo, &["worktree", "prune"]);
    if dir.exists() {
        if is_live_worktree(&dir) {
            return Ok(WorktreeRef { path: dir.to_string_lossy().into_owned(), branch, base });
        }
        let empty = std::fs::read_dir(&dir).map(|mut d| d.next().is_none()).unwrap_or(false);
        if !empty {
            return Err(format!("{} already exists and is not a worktree of this repo.", dir.display()));
        }
    }
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Couldn't create {}: {e}", parent.display()))?;
    }
    let dir_s = dir.to_string_lossy().into_owned();
    if branch_exists(repo, &branch)? {
        git::ok(repo, &["worktree", "add", &dir_s, &branch])?;
    } else {
        git::ok(repo, &["worktree", "add", "-b", &branch, &dir_s, &base])?;
    }
    let path = dir.canonicalize().map(|p| p.to_string_lossy().into_owned()).unwrap_or(dir_s);
    Ok(WorktreeRef { path, branch, base })
}

/// Deletes the worktree (even with changes) and its branch. Anything missing is ignored.
pub fn cleanup(repo: &Path, wt: &WorktreeRef) -> Result<(), String> {
    let p = Path::new(&wt.path);
    if p.exists() {
        git::ok(repo, &["worktree", "remove", "--force", &wt.path])?;
    }
    let _ = git::run(repo, &["worktree", "prune"]);
    if branch_exists(repo, &wt.branch)? {
        git::ok(repo, &["branch", "-D", &wt.branch])?;
    }
    Ok(())
}

/// State of a task's worktree, to decide whether cleaning it up is safe.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeStatus {
    /// The folder exists and is a live worktree.
    pub exists: bool,
    pub branch: Option<String>,
    pub base: Option<String>,
    /// Branch commits that aren't in the base.
    pub ahead: u32,
    /// Branch commits that are neither in the base nor in any remote: they would be lost.
    pub unpushed: u32,
    /// Uncommitted changes (including new, non-ignored files).
    pub dirty: bool,
}

/// `git rev-list --count <args> --`. An error is propagated: counting 0 on error would allow
/// deleting a branch with unpushed commits.
pub(crate) fn count(repo: &Path, args: &[&str]) -> Result<u32, String> {
    let mut full = vec!["rev-list", "--count"];
    full.extend_from_slice(args);
    full.push("--");
    let out = git::ok(repo, &full)?;
    out.trim().parse().map_err(|_| format!("Unexpected output from git rev-list: {}", out.trim()))
}

/// State of `wt` (`None`: the task has no worktree).
pub fn status(repo: &Path, wt: Option<&WorktreeRef>) -> Result<WorktreeStatus, String> {
    let Some(wt) = wt else { return Ok(WorktreeStatus::default()) };
    let exists = is_live_worktree(Path::new(&wt.path));
    let mut st = WorktreeStatus {
        exists,
        branch: Some(wt.branch.clone()),
        base: Some(wt.base.clone()),
        ..Default::default()
    };
    if branch_exists(repo, &wt.branch)? {
        let branch = format!("refs/heads/{}", wt.branch);
        let base_ok = git::run(repo, &["rev-parse", "--verify", "--quiet", &format!("{}^{{commit}}", wt.base)])?.ok;
        if base_ok {
            st.ahead = count(repo, &[&branch, "--not", &wt.base])?;
            st.unpushed = count(repo, &[&branch, "--not", &wt.base, "--remotes"])?;
        } else {
            // No base (deleted): anything not on a remote would be lost.
            st.unpushed = count(repo, &[&branch, "--not", "--remotes"])?;
            st.ahead = st.unpushed;
        }
    }
    if exists {
        st.dirty = !git::ok(Path::new(&wt.path), &["status", "--porcelain"])?.trim().is_empty();
    }
    Ok(st)
}

/// Why it can't be cleaned up without `force` (`None`: it's safe).
pub fn cleanup_blocker(st: &WorktreeStatus) -> Option<String> {
    let mut why = Vec::new();
    if st.unpushed > 0 {
        let s = if st.unpushed == 1 { "" } else { "s" };
        why.push(format!("{} unpushed commit{s}", st.unpushed));
    }
    if st.dirty {
        why.push("uncommitted changes".to_string());
    }
    (!why.is_empty()).then(|| {
        format!("The worktree has {}: push or commit them first, or clean up with force to discard them.", why.join(" and "))
    })
}

#[cfg(test)]
mod tests;
