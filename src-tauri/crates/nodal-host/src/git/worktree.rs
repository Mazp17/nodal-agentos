//! Task worktrees: `~/.nodal/worktrees/<repo>/<task-slug>` with the branch
//! `nodal/<task-slug>`, created from the repo's current branch before the first run and
//! reused by the following ones (handoffs included). All blocking.
//!
//! Naming and status types live in `nodal_domain::execution::worktree`.

use std::path::{Path, PathBuf};

use nodal_domain::execution::worktree::WorktreeStatus;
use nodal_domain::model::WorktreeRef;

use super::{ok, run};

/// The repo's current branch (or the commit if on a detached HEAD): the worktree's base.
pub fn current_base(repo: &Path) -> Result<String, String> {
    let out = run(repo, &["symbolic-ref", "--short", "-q", "HEAD"])?;
    let branch = out.stdout.trim();
    if out.ok && !branch.is_empty() {
        return Ok(branch.to_string());
    }
    Ok(ok(repo, &["rev-parse", "HEAD"])?.trim().to_string())
}

pub(crate) fn branch_exists(repo: &Path, branch: &str) -> Result<bool, String> {
    Ok(run(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )?
    .ok)
}

/// Is `dir` a live worktree (its toplevel is itself)?
pub(crate) fn is_live_worktree(dir: &Path) -> bool {
    dir.is_dir()
        && super::toplevel(dir)
            .ok()
            .flatten()
            .and_then(|t| t.canonicalize().ok())
            .zip(dir.canonicalize().ok())
            .is_some_and(|(a, b)| a == b)
}

/// Creates (or reuses) the task's worktree. `existing`: the one it already has stored.
pub fn ensure(
    repo: &Path,
    dir: &Path,
    branch: &str,
    existing: Option<&WorktreeRef>,
) -> Result<WorktreeRef, String> {
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
    let _ = run(repo, &["worktree", "prune"]);
    if dir.exists() {
        if is_live_worktree(&dir) {
            return Ok(WorktreeRef {
                path: dir.to_string_lossy().into_owned(),
                branch,
                base,
            });
        }
        let empty = std::fs::read_dir(&dir)
            .map(|mut d| d.next().is_none())
            .unwrap_or(false);
        if !empty {
            return Err(format!(
                "{} already exists and is not a worktree of this repo.",
                dir.display()
            ));
        }
    }
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Couldn't create {}: {e}", parent.display()))?;
    }
    let dir_s = dir.to_string_lossy().into_owned();
    if branch_exists(repo, &branch)? {
        ok(repo, &["worktree", "add", &dir_s, &branch])?;
    } else {
        ok(repo, &["worktree", "add", "-b", &branch, &dir_s, &base])?;
    }
    let path = dir
        .canonicalize()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or(dir_s);
    Ok(WorktreeRef { path, branch, base })
}

/// Deletes the worktree (even with changes) and its branch. Anything missing is ignored.
pub fn cleanup(repo: &Path, wt: &WorktreeRef) -> Result<(), String> {
    let p = Path::new(&wt.path);
    if p.exists() {
        ok(repo, &["worktree", "remove", "--force", &wt.path])?;
    }
    let _ = run(repo, &["worktree", "prune"]);
    if branch_exists(repo, &wt.branch)? {
        ok(repo, &["branch", "-D", &wt.branch])?;
    }
    Ok(())
}

/// `git rev-list --count <args> --`. An error is propagated: counting 0 on error would allow
/// deleting a branch with unpushed commits.
pub(crate) fn count(repo: &Path, args: &[&str]) -> Result<u32, String> {
    let mut full = vec!["rev-list", "--count"];
    full.extend_from_slice(args);
    full.push("--");
    let out = ok(repo, &full)?;
    out.trim()
        .parse()
        .map_err(|_| format!("Unexpected output from git rev-list: {}", out.trim()))
}

/// State of `wt` (`None`: the task has no worktree).
pub fn status(repo: &Path, wt: Option<&WorktreeRef>) -> Result<WorktreeStatus, String> {
    let Some(wt) = wt else {
        return Ok(WorktreeStatus::default());
    };
    let exists = is_live_worktree(Path::new(&wt.path));
    let mut st = WorktreeStatus {
        exists,
        branch: Some(wt.branch.clone()),
        base: Some(wt.base.clone()),
        ..Default::default()
    };
    if branch_exists(repo, &wt.branch)? {
        let branch = format!("refs/heads/{}", wt.branch);
        let base_ok = run(
            repo,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{}^{{commit}}", wt.base),
            ],
        )?
        .ok;
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
        st.dirty = !ok(Path::new(&wt.path), &["status", "--porcelain"])?
            .trim()
            .is_empty();
    }
    Ok(st)
}

#[cfg(test)]
mod tests;
