//! A task's pull request: found from the task's branch with `gh`, whoever opened it, and the
//! task status following it (open → In Review, merged → Done).

use std::path::Path;

use nodal_domain::model::events::ChangeKind;
use nodal_domain::model::{PrState, Task, TaskPr, TaskStatus};
use nodal_domain::ports::Git;
use nodal_domain::util::check_id;
use nodal_store::board::{repos, tasks};
use nodal_store::{Conn as Connection, Db};

use crate::core::{blocking, AppError};

use super::Board;

/// A non-forced refresh within this long of the last lookup skips `gh`.
const CHECK_INTERVAL_MS: i64 = 30_000;

/// Stores `pr` (`None`: just stamps the lookup) and moves the task after it. `true` when the PR
/// or the status changed. Run it inside a transaction.
pub fn link_pr(conn: &Connection, task: &Task, pr: Option<TaskPr>, now: i64) -> Result<bool, String> {
    tasks::set_pr(conn, &task.id, pr.as_ref(), now)?;
    let Some(pr) = pr else { return Ok(false) };
    let next = match pr.state {
        PrState::Open | PrState::Draft if matches!(task.status, TaskStatus::Todo | TaskStatus::InProgress) => Some(TaskStatus::InReview),
        PrState::Merged => Some(TaskStatus::Done),
        _ => None,
    };
    let moved = super::ops::apply_task_transition(conn, task, next, None, None, now)?.is_some();
    Ok(moved || task.pr.as_ref() != Some(&pr))
}

/// Looks up the PR of the task's branch and links it. Skipped for a task without a worktree,
/// and, unless `force`, when it was looked up less than `CHECK_INTERVAL_MS` ago. `gh` failing
/// (missing, logged out, not GitHub) is logged and leaves the task as it was. Blocking.
pub fn refresh(db: &Db, git: &dyn Git, now: i64, task_id: &str, force: bool) -> Result<(Task, bool), String> {
    let (task, repo_path, checked) = {
        let conn = db.guard();
        let task = tasks::get(&conn, task_id)?;
        let repo = repos::get(&conn, &task.repo_id)?;
        (task, repo.path, tasks::pr_checked_at(&conn, task_id)?)
    };
    let Some(wt) = task.worktree.as_ref() else { return Ok((task, false)) };
    if !force && checked.is_some_and(|at| now - at < CHECK_INTERVAL_MS) {
        return Ok((task, false));
    }
    let pr = git.pr_for_branch(Path::new(&repo_path), &wt.branch).unwrap_or_else(|e| {
        eprintln!("tasks: couldn't look up the PR of {task_id}: {e}");
        None
    });
    let mut conn = db.guard();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    // Re-read: the task may have moved while `gh` ran.
    let task = tasks::get(&tx, task_id)?;
    let changed = link_pr(&tx, &task, pr, now)?;
    let task = tasks::get(&tx, task_id)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok((task, changed))
}

/// `https://github.com/o/r/pull/12` → 12.
pub fn pr_number(url: &str) -> Option<i64> {
    url.trim_end_matches('/').rsplit('/').next()?.parse().ok()
}

impl Board {
    /// Refreshes the task's PR (see `refresh`) and notifies when it changed.
    pub async fn refresh_task_pr(&self, task_id: String, force: Option<bool>) -> Result<Task, AppError> {
        check_id(&task_id, "task")?;
        let (db, git, now) = (self.core.db.clone(), self.core.env.git.clone(), self.core.clock.now_ms());
        let (task, changed) = blocking(move || refresh(&db, git.as_ref(), now, &task_id, force.unwrap_or(false))).await?;
        if changed {
            self.core.notifier.notify(ChangeKind::Tasks, Some(&task.project_id));
        }
        Ok(task)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodal_domain::testutil::{project_of, repo_of, task_of};
    use nodal_store::rows;

    fn status_after(from: TaskStatus, state: PrState) -> (TaskStatus, bool) {
        let db = Db::open_in_memory().unwrap();
        let c = db.guard();
        rows::insert_project(&c, &project_of("p1", "PAY")).unwrap();
        rows::insert_repo(&c, &repo_of("r1", "p1", "/r1")).unwrap();
        let task = Task { status: from, ..task_of("t1") };
        rows::insert_task(&c, &task).unwrap();
        let pr = TaskPr { url: "https://github.com/o/r/pull/3".into(), number: 3, state, head: None };
        let changed = link_pr(&c, &task, Some(pr.clone()), 10).unwrap();
        let after = tasks::get(&c, "t1").unwrap();
        assert_eq!(after.pr, Some(pr));
        (after.status, changed)
    }

    #[test]
    fn the_task_follows_its_pr() {
        assert_eq!(status_after(TaskStatus::InProgress, PrState::Open), (TaskStatus::InReview, true));
        assert_eq!(status_after(TaskStatus::Todo, PrState::Draft), (TaskStatus::InReview, true));
        assert_eq!(status_after(TaskStatus::Blocked, PrState::Open), (TaskStatus::Blocked, true));
        assert_eq!(status_after(TaskStatus::InReview, PrState::Merged), (TaskStatus::Done, true));
        assert_eq!(status_after(TaskStatus::InReview, PrState::Closed), (TaskStatus::InReview, true));
        assert_eq!(status_after(TaskStatus::Canceled, PrState::Merged), (TaskStatus::Canceled, true));
    }

    #[test]
    fn a_merged_pr_moves_the_diff_base_to_its_head() {
        let wt = nodal_domain::model::WorktreeRef { path: "/w".into(), branch: "nodal/x".into(), base: "main".into() };
        let pr = |state, head: Option<&str>| Some(TaskPr { url: "u".into(), number: 1, state, head: head.map(Into::into) });
        let t = |pr| Task { worktree: Some(wt.clone()), pr, ..task_of("t1") };
        assert_eq!(t(None).diff_base(), Some("main"));
        assert_eq!(t(pr(PrState::Open, Some("abc"))).diff_base(), Some("main"));
        assert_eq!(t(pr(PrState::Merged, None)).diff_base(), Some("main"));
        assert_eq!(t(pr(PrState::Merged, Some("abc"))).diff_base(), Some("abc"));
    }

    #[test]
    fn reads_the_number_from_a_pr_url() {
        assert_eq!(pr_number("https://github.com/o/r/pull/12"), Some(12));
        assert_eq!(pr_number("https://github.com/o/r/pull/12/"), Some(12));
        assert_eq!(pr_number("nope"), None);
    }
}
