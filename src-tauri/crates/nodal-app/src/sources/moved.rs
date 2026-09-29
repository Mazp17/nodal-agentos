//! `resolve_moved`: what to do with a task whose issue changed project in the provider.
//! Moved from the shell's `providers/store.rs`; the SQL it calls now lives in
//! `nodal_store::sources`. `validate_plan_file` goes through the `PlanFiles` port (the shell's
//! `work::validate::plan_file`/`nodal_host::plans::plan_file`), since nodal-app doesn't depend
//! on nodal-host.

use std::path::Path;
use std::sync::Arc;

use nodal_domain::model::{PlanRef, Task};
use nodal_domain::ports::PlanFiles;
use nodal_domain::sources::routing::project_rule;
use nodal_store::sources::{check_repo_in_project, has_active_run, set_moved_resolution};
use nodal_store::{rows, Conn as Connection};

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
pub fn resolve_moved(
    conn: &Connection,
    plans: &Arc<dyn PlanFiles>,
    task_id: &str,
    action: MovedAction,
    now: i64,
) -> Result<Task, String> {
    let task = rows::get_task(conn, task_id)?.ok_or_else(|| "Task not found.".to_string())?;
    let Some(src) = task.source.as_ref() else {
        return Err("This task is no longer linked to its source.".into());
    };
    let Some(moved) = src.moved.as_ref() else {
        return Err("This task has no pending project change.".into());
    };
    let link = match src.link_id.as_deref() {
        Some(id) => rows::get_source_link(conn, id)?,
        None => None,
    };
    let repo_id = match action {
        MovedAction::Keep => task.repo_id.clone(),
        MovedAction::Move => {
            let Some(target) = moved.suggested_repo_id.clone() else {
                return Err("No repo is suggested for its new project: keep it or move it by hand.".into());
            };
            if target != task.repo_id {
                check_repo_in_project(conn, &target, &task.project_id)?;
                if has_active_run(conn, &task.id)? {
                    return Err(
                        "The task has a queued or running run: wait for it or cancel it before moving it to another repo."
                            .into(),
                    );
                }
                if task.worktree.is_some() {
                    return Err("Clean up the task's worktree before moving it to another repo.".into());
                }
                if let PlanRef::File { path } = &task.plan {
                    let repo = rows::get_repo(conn, &target)?.ok_or_else(|| "Repo not found.".to_string())?;
                    plans.validate_plan_file(Path::new(&repo.path), path).map_err(|_| {
                        "The plan file is in the old repo: pick a new plan for this task first.".to_string()
                    })?;
                }
            }
            target
        }
    };
    let current = moved.to_project.as_ref().map(|p| p.id.as_str());
    let rule_id = link.as_ref().and_then(|l| project_rule(l, current)).filter(|r| r.repo_id == repo_id).map(|r| r.id.clone());
    set_moved_resolution(conn, &task.id, &repo_id, rule_id.as_deref(), now)?;
    rows::get_task(conn, &task.id)?.ok_or_else(|| "Task not found.".to_string())
}
