//! "Merge into <base> & done": lands a task branch on the branch its worktree started from
//! without checking anything out in the main repo. The base only ever moves by fast-forward;
//! if it moved since the task started, it's merged into the task's worktree first. A conflict
//! is aborted there, so no checkout is left mid-merge. All blocking.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::domain::WorktreeRef;
use crate::util::git;

use super::worktree::{branch_exists, count, is_live_worktree};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MergeOutcome {
    /// `commit`: the base's new tip. `commits`: task commits that landed. `moved`: the base had
    /// moved and was merged into the worktree first.
    Merged { commit: String, commits: u32, squashed: bool, moved: bool },
    /// Merging the base into the worktree conflicted in `files`; the merge was aborted.
    Conflict { files: Vec<String> },
}

/// What `merge_worktree` did. `task` is the task after it (Done unless it conflicted).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeReport {
    pub outcome: MergeOutcome,
    pub task: crate::domain::Task,
    /// Remote the base was pushed to (only when asked).
    pub pushed_to: Option<String>,
    pub push_error: Option<String>,
    /// Why the worktree couldn't be cleaned up; the merge itself went through.
    pub cleanup_error: Option<String>,
}

/// Worktree that has `branch` checked out (the main repo included), if any.
fn checkout_of(repo: &Path, branch: &str) -> Result<Option<PathBuf>, String> {
    let list = git::ok(repo, &["worktree", "list", "--porcelain"])?;
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
    Ok(!git::ok(dir, &["status", "--porcelain", "--untracked-files=no"])?.trim().is_empty())
}

fn rev(dir: &Path, rev: &str) -> Result<String, String> {
    Ok(git::ok(dir, &["rev-parse", "--verify", "--quiet", rev])?.trim().to_string())
}

/// Lands `wt.branch` on `wt.base`. Uncommitted changes in the worktree are committed first with
/// `message`, which is also the squash commit's message. Nothing is touched when it refuses.
pub fn merge(repo: &Path, wt: &WorktreeRef, message: &str, squash: bool) -> Result<MergeOutcome, String> {
    let (base, branch) = (wt.base.as_str(), wt.branch.as_str());
    let wt_dir = Path::new(&wt.path);
    if !branch_exists(repo, base)? {
        return Err(format!("The task started from {base}, which isn't a local branch: there's nothing to merge into."));
    }
    if !is_live_worktree(wt_dir) {
        return Err(format!("The task's worktree is gone ({}): nothing to merge from.", wt.path));
    }
    if git::run(wt_dir, &["rev-parse", "--verify", "--quiet", "MERGE_HEAD"])?.ok {
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

    if !git::ok(wt_dir, &["status", "--porcelain"])?.trim().is_empty() {
        git::ok(wt_dir, &["add", "-A"])?;
        git::ok(wt_dir, &["commit", "-q", "-m", message])?;
    }
    let base_ref = format!("refs/heads/{base}");
    let branch_ref = format!("refs/heads/{branch}");
    let commits = count(repo, &[&branch_ref, "--not", &base_ref])?;
    if commits == 0 {
        return Err(format!("{branch} has nothing that isn't already in {base}."));
    }
    let base_tip = rev(repo, &base_ref)?;
    let moved = !git::run(repo, &["merge-base", "--is-ancestor", &base_tip, &branch_ref])?.ok;
    if moved {
        let msg = format!("Merge {base} into {branch}");
        let merged = git::run(wt_dir, &["merge", "-q", "--no-edit", "-m", &msg, &base_tip])?;
        if !merged.ok {
            let files: Vec<String> = git::ok(wt_dir, &["diff", "--name-only", "--diff-filter=U"])?
                .lines()
                .map(str::to_string)
                .collect();
            let _ = git::run(wt_dir, &["merge", "--abort"]);
            if files.is_empty() {
                let why = if merged.stderr.trim().is_empty() { merged.stdout } else { merged.stderr };
                return Err(format!("Couldn't merge {base} into {branch}: {}", crate::util::clip_chars(why.trim(), 600)));
            }
            return Ok(MergeOutcome::Conflict { files });
        }
    }
    let head = rev(wt_dir, "HEAD")?;
    let target = if squash {
        let tree = format!("{head}^{{tree}}");
        git::ok(repo, &["commit-tree", &tree, "-p", &base_tip, "-m", message])?.trim().to_string()
    } else {
        head
    };
    match &checkout {
        Some(dir) => git::ok(dir, &["merge", "-q", "--ff-only", &target]).map(drop)?,
        None => git::ok(repo, &["update-ref", "-m", &format!("nodal: merge {branch}"), &base_ref, &target, &base_tip]).map(drop)?,
    }
    if squash {
        // Same tree, so nothing in the worktree changes; the branch stops looking unmerged.
        git::ok(wt_dir, &["reset", "-q", "--keep", &target])?;
    }
    Ok(MergeOutcome::Merged { commit: target, commits, squashed: squash, moved })
}

/// Pushes `base` to its upstream (or to `origin` under the same name). Returns the remote.
pub fn push_base(repo: &Path, base: &str) -> Result<String, String> {
    let config = |key: String| -> Result<Option<String>, String> {
        let out = git::run(repo, &["config", "--get", &key])?;
        Ok(Some(out.stdout.trim().to_string()).filter(|s| out.ok && !s.is_empty()))
    };
    let remote = config(format!("branch.{base}.remote"))?.filter(|r| r != ".").unwrap_or_else(|| "origin".into());
    let dst = config(format!("branch.{base}.merge"))?.unwrap_or_else(|| format!("refs/heads/{base}"));
    git::ok(repo, &["push", "-q", "--", &remote, &format!("refs/heads/{base}:{dst}")])?;
    Ok(remote)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::paths::tests::{git_available, init_repo, TempDir};
    use crate::work::worktree::{self, cleanup, cleanup_blocker, dir_for, ensure, status};

    const MSG: &str = "PLA-2: tarea 2";

    fn setup(name: &str) -> Option<(TempDir, PathBuf, WorktreeRef)> {
        if !git_available() {
            eprintln!("git not available: skipping");
            return None;
        }
        let t = TempDir::new(name);
        let repo = t.0.join("repo");
        init_repo(&repo);
        let wt = ensure(&repo, &dir_for(&t.0.join("worktrees"), "repo", "pla-2"), "nodal/pla-2", None).unwrap();
        Some((t, repo, wt))
    }

    fn commit(dir: &Path, file: &str, body: &str, msg: &str) {
        std::fs::write(dir.join(file), body).unwrap();
        git::ok(dir, &["add", "."]).unwrap();
        git::ok(dir, &["commit", "-q", "-m", msg]).unwrap();
    }

    fn tip(dir: &Path, r: &str) -> String {
        rev(dir, r).unwrap()
    }

    fn subject(dir: &Path, r: &str) -> String {
        git::ok(dir, &["log", "-1", "--format=%s", r]).unwrap().trim().to_string()
    }

    #[test]
    fn fast_forwards_an_unmoved_base_without_touching_its_checkout() {
        let Some((_t, repo, wt)) = setup("merge-ff") else { return };
        // The main folder sits on another branch: merging must not switch it.
        git::ok(&repo, &["checkout", "-q", "-b", "other"]).unwrap();
        let w = Path::new(&wt.path);
        commit(w, "a.txt", "a", "first");
        commit(w, "b.txt", "b", "second");
        let head = tip(w, "HEAD");

        let out = merge(&repo, &wt, MSG, false).unwrap();
        assert_eq!(out, MergeOutcome::Merged { commit: head.clone(), commits: 2, squashed: false, moved: false });
        assert_eq!(tip(&repo, "refs/heads/main"), head);
        assert_eq!(worktree::current_base(&repo).unwrap(), "other");
        assert!(!repo.join("a.txt").exists());
        let st = status(&repo, Some(&wt)).unwrap();
        assert_eq!((st.ahead, st.unpushed), (0, 0));
    }

    #[test]
    fn squashes_with_the_task_as_message_and_keeps_the_checked_out_base_in_sync() {
        let Some((_t, repo, wt)) = setup("merge-squash") else { return };
        let w = Path::new(&wt.path);
        let before = tip(&repo, "main");
        commit(w, "a.txt", "a", "first");
        commit(w, "b.txt", "b", "second");

        let out = merge(&repo, &wt, MSG, true).unwrap();
        let MergeOutcome::Merged { commit, commits, squashed, moved } = out else { panic!("{out:?}") };
        assert_eq!((commits, squashed, moved), (2, true, false));
        assert_eq!(tip(&repo, "main"), commit);
        assert_eq!(tip(&repo, "main~1"), before);
        assert_eq!(subject(&repo, "main"), MSG);
        // `main` is checked out in the repo folder: its files follow the new tip.
        assert!(repo.join("a.txt").is_file() && repo.join("b.txt").is_file());
        assert!(!has_tracked_changes(&repo).unwrap());
        // The task branch now points at the squash commit: clean up needs no force.
        assert_eq!(tip(w, "HEAD"), commit);
        assert_eq!(cleanup_blocker(&status(&repo, Some(&wt)).unwrap()), None);
        cleanup(&repo, &wt).unwrap();
    }

    #[test]
    fn commits_uncommitted_changes_first() {
        let Some((_t, repo, wt)) = setup("merge-dirty-wt") else { return };
        std::fs::write(Path::new(&wt.path).join("new.txt"), "x").unwrap();
        let out = merge(&repo, &wt, MSG, false).unwrap();
        assert!(matches!(out, MergeOutcome::Merged { commits: 1, .. }), "{out:?}");
        assert_eq!(subject(&repo, "main"), MSG);
        assert!(repo.join("new.txt").is_file());
    }

    #[test]
    fn merges_a_moved_base_into_the_worktree_first() {
        let Some((_t, repo, wt)) = setup("merge-moved") else { return };
        commit(&repo, "base.txt", "b", "base moved");
        let base_tip = tip(&repo, "main");
        let w = Path::new(&wt.path);
        commit(w, "task.txt", "t", "task work");

        let out = merge(&repo, &wt, MSG, false).unwrap();
        let MergeOutcome::Merged { commit, moved, .. } = out else { panic!("{out:?}") };
        assert!(moved);
        // The worktree got the base merged in, and the base only fast-forwarded to it.
        assert!(w.join("base.txt").is_file());
        assert_eq!(tip(w, "HEAD"), commit);
        assert_eq!(tip(&repo, "main"), commit);
        assert!(git::run(&repo, &["merge-base", "--is-ancestor", &base_tip, "main"]).unwrap().ok);
        assert!(repo.join("task.txt").is_file());
    }

    #[test]
    fn squashes_on_top_of_a_moved_base() {
        let Some((_t, repo, wt)) = setup("merge-moved-squash") else { return };
        commit(&repo, "base.txt", "b", "base moved");
        let base_tip = tip(&repo, "main");
        commit(Path::new(&wt.path), "task.txt", "t", "task work");

        let out = merge(&repo, &wt, MSG, true).unwrap();
        assert!(matches!(out, MergeOutcome::Merged { moved: true, squashed: true, .. }), "{out:?}");
        assert_eq!(tip(&repo, "main~1"), base_tip);
        assert_eq!(subject(&repo, "main"), MSG);
        assert!(repo.join("task.txt").is_file() && repo.join("base.txt").is_file());
    }

    #[test]
    fn stops_when_the_checked_out_base_has_uncommitted_changes() {
        let Some((_t, repo, wt)) = setup("merge-dirty-base") else { return };
        commit(Path::new(&wt.path), "a.txt", "a", "first");
        std::fs::write(repo.join("README.md"), "# edited by hand\n").unwrap();
        let before = tip(&repo, "main");

        let err = merge(&repo, &wt, MSG, true).unwrap_err();
        assert!(err.contains("uncommitted changes") && err.contains("main"), "{err}");
        assert_eq!(tip(&repo, "main"), before);
        assert_eq!(std::fs::read_to_string(repo.join("README.md")).unwrap(), "# edited by hand\n");
        // Untracked files alone don't block it.
        git::ok(&repo, &["checkout", "--", "README.md"]).unwrap();
        std::fs::write(repo.join("scratch.txt"), "x").unwrap();
        assert!(matches!(merge(&repo, &wt, MSG, true).unwrap(), MergeOutcome::Merged { .. }));
    }

    #[test]
    fn aborts_on_conflict_and_leaves_everything_as_it_was() {
        let Some((_t, repo, wt)) = setup("merge-conflict") else { return };
        commit(&repo, "README.md", "# base\n", "base edit");
        let base_tip = tip(&repo, "main");
        let w = Path::new(&wt.path);
        commit(w, "README.md", "# task\n", "task edit");
        let head = tip(w, "HEAD");

        let out = merge(&repo, &wt, MSG, true).unwrap();
        assert_eq!(out, MergeOutcome::Conflict { files: vec!["README.md".into()] });
        assert_eq!(tip(&repo, "main"), base_tip);
        assert_eq!(tip(w, "HEAD"), head);
        assert!(!git::run(w, &["rev-parse", "--verify", "--quiet", "MERGE_HEAD"]).unwrap().ok);
        assert!(git::ok(w, &["status", "--porcelain"]).unwrap().trim().is_empty());
    }

    #[test]
    fn refuses_with_nothing_to_merge_or_no_base_branch() {
        let Some((_t, repo, wt)) = setup("merge-nothing") else { return };
        assert!(merge(&repo, &wt, MSG, true).unwrap_err().contains("nothing"));
        let detached = WorktreeRef { base: tip(&repo, "main"), ..wt.clone() };
        assert!(merge(&repo, &detached, MSG, true).unwrap_err().contains("isn't a local branch"));
    }

    #[test]
    fn pushes_the_base_to_its_remote_only_when_asked() {
        let Some((t, repo, wt)) = setup("merge-push") else { return };
        let remote = t.0.join("remote.git");
        git::ok(&t.0, &["init", "-q", "--bare", remote.to_str().unwrap()]).unwrap();
        git::ok(&repo, &["remote", "add", "origin", remote.to_str().unwrap()]).unwrap();
        git::ok(&repo, &["push", "-q", "-u", "origin", "main"]).unwrap();
        let pushed_before = tip(&remote, "refs/heads/main");
        commit(Path::new(&wt.path), "a.txt", "a", "first");

        let MergeOutcome::Merged { commit, .. } = merge(&repo, &wt, MSG, true).unwrap() else { panic!() };
        assert_eq!(tip(&remote, "refs/heads/main"), pushed_before);
        assert_eq!(push_base(&repo, "main").unwrap(), "origin");
        assert_eq!(tip(&remote, "refs/heads/main"), commit);
    }

}
