//! `resolve_moved`: business logic for a task whose issue changed project (kept here until
//! W3b moves it to `app::sources::moved`). The SQL now lives in `nodal_store::sources`,
//! re-exported below so current uses don't break.

pub use nodal_store::sources::*;

use crate::db::{rows, Connection, DbError};
use crate::domain::{PlanRef, Task};

/// What to do with a task whose issue changed project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MovedAction {
    /// Move it to the suggested repo.
    Move,
    /// Keep it in its repo.
    Keep,
}

/// Resolves the project-change warning. `Move` rejects with an active run, a worktree or a
/// plan file outside the new repo. In both cases the task gets tied to the current project's
/// rule if that rule points to its repo (so the next pull doesn't warn again about the same
/// change).
pub fn resolve_moved(conn: &Connection, task_id: &str, action: MovedAction, now: i64) -> Result<Task, DbError> {
    let task = rows::get_task(conn, task_id)?.ok_or_else(|| DbError::Invalid("Task not found.".into()))?;
    let Some(src) = task.source.as_ref() else {
        return Err(DbError::Invalid("This task is no longer linked to its source.".into()));
    };
    let Some(moved) = src.moved.as_ref() else {
        return Err(DbError::Invalid("This task has no pending project change.".into()));
    };
    let link = match src.link_id.as_deref() {
        Some(id) => rows::get_source_link(conn, id)?,
        None => None,
    };
    let repo_id = match action {
        MovedAction::Keep => task.repo_id.clone(),
        MovedAction::Move => {
            let Some(target) = moved.suggested_repo_id.clone() else {
                return Err(DbError::Invalid("No repo is suggested for its new project: keep it or move it by hand.".into()));
            };
            if target != task.repo_id {
                check_repo_in_project(conn, &target, &task.project_id)?;
                if has_active_run(conn, &task.id)? {
                    return Err(DbError::Invalid(
                        "The task has a queued or running run: wait for it or cancel it before moving it to another repo."
                            .into(),
                    ));
                }
                if task.worktree.is_some() {
                    return Err(DbError::Invalid(
                        "Clean up the task's worktree before moving it to another repo.".into(),
                    ));
                }
                if let PlanRef::File { path } = &task.plan {
                    let repo = rows::get_repo(conn, &target)?.ok_or_else(|| DbError::Invalid("Repo not found.".into()))?;
                    crate::work::validate::plan_file(std::path::Path::new(&repo.path), path).map_err(|_| {
                        DbError::Invalid("The plan file is in the old repo: pick a new plan for this task first.".into())
                    })?;
                }
            }
            target
        }
    };
    let current = moved.to_project.as_ref().map(|p| p.id.as_str());
    let rule_id = link
        .as_ref()
        .and_then(|l| super::import::project_rule(l, current))
        .filter(|r| r.repo_id == repo_id)
        .map(|r| r.id.clone());
    set_moved_resolution(conn, &task.id, &repo_id, rule_id.as_deref(), now)?;
    rows::get_task(conn, &task.id)?.ok_or_else(|| DbError::Invalid("Task not found.".into()))
}
