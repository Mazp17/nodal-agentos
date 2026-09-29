//! Worktree status, cleanup and merge. Moved from `commands::execution::{worktree_status,
//! cleanup_worktree, merge_worktree, forget_worktree}`. Git operations now go through the
//! injected `Git` port instead of `nodal_host::git::worktree`/`merge` directly.

use std::path::{Path, PathBuf};

use nodal_domain::board::dto::{MergeInput, TaskPatch};
use nodal_domain::execution::worktree::{cleanup_blocker, MergeOutcome, MergeReport, WorktreeStatus};
use nodal_domain::model::events::ChangeKind;
use nodal_domain::model::{task_key, Task, TaskStatus};
use nodal_domain::util::check_id;
use nodal_store::board::{projects, repos, tasks};
use nodal_store::execution::runs as qruns;
use nodal_store::with_db;

use crate::core::{blocking, AppError};

use super::enqueue::lock;
use super::Execution;

const PENDING_ERR: &str = "The task has a queued or running run: cancel it first.";

impl Execution {
    pub async fn worktree_status(&self, task_id: String) -> Result<WorktreeStatus, AppError> {
        check_id(&task_id, "task")?;
        let (task, repo) = with_db(&self.core.db, move |c| {
            let t = tasks::get(c, &task_id)?;
            let r = repos::get(c, &t.repo_id)?;
            Ok((t, r))
        })
        .await?;
        let git = self.core.env.git.clone();
        Ok(blocking(move || git.worktree_status(Path::new(&repo.path), task.worktree.as_ref()).map_err(|e| e.to_string()))
            .await?)
    }

    /// "Clean up": deletes the task's worktree and branch. Refuses with runs in progress and,
    /// without `force`, if there are unpushed commits or uncommitted changes.
    ///
    /// The task stays marked in `cleaning` (in the same section that checks for pending runs,
    /// while holding the database) until the end: meanwhile `enqueue_work` rejects it, so a
    /// launch doesn't reuse the worktree being deleted.
    pub async fn cleanup_worktree(&self, task_id: String, force: Option<bool>) -> Result<Task, AppError> {
        check_id(&task_id, "task")?;
        let force = force.unwrap_or(false);
        let (id, cleaning) = (task_id.clone(), self.cleaning.clone());
        let (task, repo, _guard) = with_db(&self.core.db, move |c| {
            let t = tasks::get(c, &id)?;
            let guard = cleaning
                .mark(&id)
                .ok_or_else(|| nodal_store::StoreError::Invalid("The task's worktree is already being cleaned up.".into()))?;
            if !qruns::pending_for_task(c, &id)?.is_empty() {
                return Err(nodal_store::StoreError::Invalid(PENDING_ERR.into()));
            }
            let r = repos::get(c, &t.repo_id)?;
            Ok((t, r, guard))
        })
        .await?;
        let Some(wt) = task.worktree.clone() else { return Ok(task) };
        let db = self.core.db.clone();
        let git = self.core.env.git.clone();
        let id = task_id.clone();
        blocking(move || {
            let repo = Path::new(&repo.path);
            if !force {
                if let Some(why) = cleanup_blocker(&git.worktree_status(repo, Some(&wt)).map_err(|e| e.to_string())?) {
                    return Err(why);
                }
            }
            // With the mark set no new run can appear; checked again just in case.
            if !qruns::pending_for_task(&lock(&db), &id)?.is_empty() {
                return Err(PENDING_ERR.into());
            }
            git.cleanup_worktree(repo, &wt).map_err(|e| e.to_string())
        })
        .await?;
        let t = self.forget_worktree(task_id).await?;
        self.core.notifier.notify(ChangeKind::Tasks, Some(&t.project_id));
        Ok(t)
    }

    async fn forget_worktree(&self, task_id: String) -> Result<Task, AppError> {
        let now = self.core.clock.now_ms();
        Ok(with_db(&self.core.db, move |c| {
            let mut t = tasks::get(c, &task_id)?;
            t.worktree = None;
            t.updated_at = now;
            tasks::update(c, &t)?;
            Ok(t)
        })
        .await?)
    }

    /// "Merge into <base> & done": lands the task branch on its base, marks the task Done
    /// (which queues the status for the linked task manager) and, if asked, pushes the base
    /// and cleans up the worktree. A failed push or cleanup doesn't undo the merge: it comes
    /// back in the report. The `cleaning` mark keeps runs out of the worktree meanwhile.
    pub async fn merge_worktree(&self, task_id: String, input: MergeInput) -> Result<MergeReport, AppError> {
        check_id(&task_id, "task")?;
        let (id, cleaning) = (task_id.clone(), self.cleaning.clone());
        let (task, repo, message, _guard) = with_db(&self.core.db, move |c| {
            let t = tasks::get(c, &id)?;
            let guard = cleaning
                .mark(&id)
                .ok_or_else(|| nodal_store::StoreError::Invalid("The task's worktree is busy: wait a moment and try again.".into()))?;
            if !qruns::pending_for_task(c, &id)?.is_empty() {
                return Err(nodal_store::StoreError::Invalid(PENDING_ERR.into()));
            }
            let r = repos::get(c, &t.repo_id)?;
            let p = projects::get(c, &t.project_id)?;
            let message = format!("{}: {}", task_key(&p.key, t.number), t.title);
            Ok((t, r, message, guard))
        })
        .await?;
        let wt = task.worktree.clone().ok_or("The task has no worktree to merge.")?;
        let repo_path = PathBuf::from(&repo.path);
        let (rp, w, squash) = (repo_path.clone(), wt.clone(), input.squash);
        let git = self.core.env.git.clone();
        let outcome = blocking({
            let git = git.clone();
            move || git.merge(&rp, &w, &message, squash).map_err(|e| e.to_string())
        })
        .await?;
        if matches!(outcome, MergeOutcome::Conflict { .. }) {
            return Ok(MergeReport { outcome, task, pushed_to: None, push_error: None, cleanup_error: None });
        }
        let (mut pushed_to, mut push_error) = (None, None);
        if input.push {
            let (rp, base) = (repo_path.clone(), wt.base.clone());
            let git = git.clone();
            match blocking(move || git.push_base(&rp, &base).map_err(|e| e.to_string())).await {
                Ok(remote) => pushed_to = Some(remote),
                Err(e) => push_error = Some(e),
            }
        }
        let (env, id) = (self.core.env.clone(), task_id.clone());
        let now = self.core.clock.now_ms();
        let mut task = with_db(&self.core.db, move |c| {
            let done = TaskPatch { status: Some(TaskStatus::Done), ..Default::default() };
            crate::board::ops::update_task(c, &env, &id, &done, now).map_err(nodal_store::StoreError::Invalid)
        })
        .await?;
        let mut cleanup_error = None;
        if input.cleanup {
            let cleaned = blocking(move || {
                if let Some(why) = cleanup_blocker(&git.worktree_status(&repo_path, Some(&wt)).map_err(|e| e.to_string())?) {
                    return Err(why);
                }
                git.cleanup_worktree(&repo_path, &wt).map_err(|e| e.to_string())
            })
            .await;
            match cleaned {
                Ok(()) => task = self.forget_worktree(task_id).await?,
                Err(e) => cleanup_error = Some(e),
            }
        }
        self.core.notifier.notify(ChangeKind::Tasks, Some(&task.project_id));
        Ok(MergeReport { outcome, task, pushed_to, push_error, cleanup_error })
    }
}
