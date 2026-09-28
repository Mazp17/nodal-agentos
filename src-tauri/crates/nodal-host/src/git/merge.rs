//! "Merge into <base> & done": lands a task branch on the branch its worktree started from
//! without checking anything out in the main repo. The base only ever moves by fast-forward;
//! if it moved since the task started, it's merged into the task's worktree first. A conflict
//! is aborted there, so no checkout is left mid-merge. All blocking.

use std::path::{Path, PathBuf};

use nodal_domain::execution::worktree::MergeOutcome;
use nodal_domain::model::WorktreeRef;

use super::worktree::{branch_exists, count, is_live_worktree};
use super::{ok, run};

/// Worktree that has `branch` checked out (the main repo included), if any.
fn checkout_of(repo: &Path, branch: &str) -> Result<Option<PathBuf>, String> {
    let list = ok(repo, &["worktree", "list", "--porcelain"])?;
    let target = format!("branch refs/heads/{branch}");
    let mut path: Option<&str> = None;
    for line in list.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            path = Some(p);
        } else if line == target {
            return Ok(path.map(PathBuf::from));
        }
    }
    Ok(None)
}

/// Tracked changes only: untracked files can't be clobbered, `merge --ff-only` refuses first.
fn has_tracked_changes(dir: &Path) -> Result<bool, String> {
    Ok(
        !ok(dir, &["status", "--porcelain", "--untracked-files=no"])?
            .trim()
            .is_empty(),
    )
}

fn rev(dir: &Path, rev: &str) -> Result<String, String> {
    Ok(ok(dir, &["rev-parse", "--verify", "--quiet", rev])?
        .trim()
        .to_string())
}

/// Lands `wt.branch` on `wt.base`. Uncommitted changes in the worktree are committed first with
/// `message`, which is also the squash commit's message. Nothing is touched when it refuses.
pub fn merge(
    repo: &Path,
    wt: &WorktreeRef,
    message: &str,
    squash: bool,
) -> Result<MergeOutcome, String> {
    let (base, branch) = (wt.base.as_str(), wt.branch.as_str());
    let wt_dir = Path::new(&wt.path);
    if !branch_exists(repo, base)? {
        return Err(format!("The task started from {base}, which isn't a local branch: there's nothing to merge into."));
    }
    if !is_live_worktree(wt_dir) {
        return Err(format!(
            "The task's worktree is gone ({}): nothing to merge from.",
            wt.path
        ));
    }
    if run(wt_dir, &["rev-parse", "--verify", "--quiet", "MERGE_HEAD"])?.ok {
        return Err("The worktree is in the middle of a merge: finish or abort it first.".into());
    }
    let checkout = checkout_of(repo, base)?;
    if let Some(dir) = &checkout {
        if has_tracked_changes(dir)? {
            return Err(format!(
                "{base} is checked out in {} with uncommitted changes: commit or stash them first. Nothing was merged.",
                dir.display()
            ));
        }
    }

    if !ok(wt_dir, &["status", "--porcelain"])?.trim().is_empty() {
        ok(wt_dir, &["add", "-A"])?;
        ok(wt_dir, &["commit", "-q", "-m", message])?;
    }
    let base_ref = format!("refs/heads/{base}");
    let branch_ref = format!("refs/heads/{branch}");
    let commits = count(repo, &[&branch_ref, "--not", &base_ref])?;
    if commits == 0 {
        return Err(format!(
            "{branch} has nothing that isn't already in {base}."
        ));
    }
    let base_tip = rev(repo, &base_ref)?;
    let moved = !run(
        repo,
        &["merge-base", "--is-ancestor", &base_tip, &branch_ref],
    )?
    .ok;
    if moved {
        let msg = format!("Merge {base} into {branch}");
        let merged = run(wt_dir, &["merge", "-q", "--no-edit", "-m", &msg, &base_tip])?;
        if !merged.ok {
            let files: Vec<String> = ok(wt_dir, &["diff", "--name-only", "--diff-filter=U"])?
                .lines()
                .map(str::to_string)
                .collect();
            let _ = run(wt_dir, &["merge", "--abort"]);
            if files.is_empty() {
                let why = if merged.stderr.trim().is_empty() {
                    merged.stdout
                } else {
                    merged.stderr
                };
                return Err(format!(
                    "Couldn't merge {base} into {branch}: {}",
                    nodal_domain::util::clip_chars(why.trim(), 600)
                ));
            }
            return Ok(MergeOutcome::Conflict { files });
        }
    }
    let head = rev(wt_dir, "HEAD")?;
    let target = if squash {
        let tree = format!("{head}^{{tree}}");
        ok(
            repo,
            &["commit-tree", &tree, "-p", &base_tip, "-m", message],
        )?
        .trim()
        .to_string()
    } else {
        head
    };
    match &checkout {
        Some(dir) => ok(dir, &["merge", "-q", "--ff-only", &target]).map(drop)?,
        None => ok(
            repo,
            &[
                "update-ref",
                "-m",
                &format!("nodal: merge {branch}"),
                &base_ref,
                &target,
                &base_tip,
            ],
        )
        .map(drop)?,
    }
    if squash {
        // Same tree, so nothing in the worktree changes; the branch stops looking unmerged.
        ok(wt_dir, &["reset", "-q", "--keep", &target])?;
    }
    Ok(MergeOutcome::Merged {
        commit: target,
        commits,
        squashed: squash,
        moved,
    })
}

/// Pushes `base` to its upstream (or to `origin` under the same name). Returns the remote.
pub fn push_base(repo: &Path, base: &str) -> Result<String, String> {
    let config = |key: String| -> Result<Option<String>, String> {
        let out = run(repo, &["config", "--get", &key])?;
        Ok(Some(out.stdout.trim().to_string()).filter(|s| out.ok && !s.is_empty()))
    };
    let remote = config(format!("branch.{base}.remote"))?
        .filter(|r| r != ".")
        .unwrap_or_else(|| "origin".into());
    let dst =
        config(format!("branch.{base}.merge"))?.unwrap_or_else(|| format!("refs/heads/{base}"));
    ok(
        repo,
        &[
            "push",
            "-q",
            "--",
            &remote,
            &format!("refs/heads/{base}:{dst}"),
        ],
    )?;
    Ok(remote)
}

#[cfg(test)]
mod tests;
