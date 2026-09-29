//! A run's diff: `git diff <base>...HEAD` in its folder, plus anything uncommitted, or (for a
//! workflow) the branch it worked on inspected from the repo.
//!
//! Moved from `commands::execution::{run_diff, run_cwd, diff_base}`. Git operations now go
//! through the injected `Git` port instead of `nodal_host::git::diff` directly.

use std::path::{Path, PathBuf};

use nodal_domain::diff::{parse, RunDiff};
use nodal_domain::model::{Executor, Isolation, Run, RunStatus, Task};
use nodal_domain::util::check_id;
use nodal_store::execution::runs as qruns;
use nodal_store::rows;
use nodal_store::with_db;

use crate::core::{blocking, AppError};

use super::Execution;

/// Diff base of a run: the task worktree's base if the run ran there.
pub(super) fn diff_base(task: Option<&Task>, run: &Run) -> Option<String> {
    let wt = task.and_then(|t| t.worktree.as_ref())?;
    (run.isolation == Some(Isolation::Worktree) && Path::new(&wt.path) == Path::new(&run.cwd)).then(|| wt.base.clone())
}

impl Execution {
    pub async fn run_diff(&self, run_id: String) -> Result<RunDiff, AppError> {
        check_id(&run_id, "run")?;
        let (run, task, repo) = with_db(&self.core.db, move |c| {
            let r = qruns::get(c, &run_id)?;
            let t = match &r.task_id {
                Some(t) => rows::get_task(c, t)?,
                None => None,
            };
            let repo = match &r.repo_id {
                Some(id) => rows::get_repo(c, id)?,
                None => None,
            };
            Ok((r, t, repo))
        })
        .await?;
        let live = matches!(run.status, RunStatus::Launching | RunStatus::Launched);
        let git = self.core.env.git.clone();
        let fs = self.core.env.fs.clone();
        Ok(blocking(move || {
            // A workflow works in its own worktree: its branch is inspected from the repo.
            if let (Executor::Workflow { .. }, Some(branch), Some(repo)) = (&run.executor, &run.branch, &repo) {
                let repo_dir = Path::new(&repo.path);
                let base = git.current_base(repo_dir).map_err(|e| e.to_string())?;
                let patch = git.diff_branch(repo_dir, &base, branch).map_err(|e| e.to_string())?;
                let commits = git.commits(repo_dir, &base, branch).unwrap_or_default();
                return Ok(RunDiff {
                    branch: Some(branch.clone()),
                    commits,
                    live,
                    base,
                    cwd: repo.path.clone(),
                    includes_working_tree: false,
                    files: parse(&patch),
                    patch,
                });
            }
            let cwd = Path::new(&run.cwd);
            if !fs.is_dir(cwd) {
                return Err(format!("The run's folder no longer exists: {}", run.cwd));
            }
            let base = diff_base(task.as_ref(), &run);
            let (patch, dirty) = git.diff(cwd, base.as_deref()).map_err(|e| e.to_string())?;
            let commits = match &base {
                Some(b) => git.commits(cwd, b, "HEAD").unwrap_or_default(),
                None => Vec::new(),
            };
            Ok(RunDiff {
                branch: git.current_branch(cwd),
                commits,
                live,
                base: base.unwrap_or_else(|| "HEAD".into()),
                cwd: run.cwd.clone(),
                includes_working_tree: dirty,
                files: parse(&patch),
                patch,
            })
        })
        .await?)
    }

    /// The run's working folder, if it still exists on disk. Not a Tauri command itself:
    /// `open_worktree`/`open_in_editor` (shell-only I/O; they spawn processes) call it to read
    /// the run through this facade instead of the database directly.
    pub async fn run_cwd(&self, run_id: String) -> Result<PathBuf, AppError> {
        check_id(&run_id, "run")?;
        let cwd = with_db(&self.core.db, move |c| Ok(qruns::get(c, &run_id)?.cwd)).await?;
        let p = PathBuf::from(&cwd);
        if !self.core.env.fs.is_dir(&p) {
            return Err(format!("The run's folder no longer exists: {cwd}").into());
        }
        Ok(p)
    }
}
