//! Worktree Tauri commands (signatures in `src/domain/api.ts`). Thin: parse, then
//! `state.board.<method>(…)`.

use std::sync::Arc;

use tauri::State;

use nodal_app::App;
use nodal_domain::board::dto::MergeInput;
use nodal_domain::execution::worktree::{MergeReport, WorktreeStatus};
use nodal_domain::model::Task;

use crate::commands::CommandError;

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn worktree_status(state: State<'_, Arc<App>>, task_id: String) -> Result<WorktreeStatus, CommandError> {
    Ok(state.board.worktree_status(task_id).await?)
}

/// "Clean up": deletes the task's worktree and branch. Without `force`, refuses if there are
/// unpushed commits or uncommitted changes.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn cleanup_worktree(state: State<'_, Arc<App>>, task_id: String, force: Option<bool>) -> Result<Task, CommandError> {
    Ok(state.board.cleanup_worktree(task_id, force).await?)
}

/// "Merge into <base> & done": lands the task branch on its base, marks the task Done and, if
/// asked, pushes the base and cleans up the worktree.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn merge_worktree(state: State<'_, Arc<App>>, task_id: String, input: MergeInput) -> Result<MergeReport, CommandError> {
    Ok(state.board.merge_worktree(task_id, input).await?)
}

/// Deletes one of a repo's linked worktrees and its branch. Without `force`, refuses
/// uncommitted changes and unpushed commits. The main checkout is refused.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn remove_repo_worktree(state: State<'_, Arc<App>>, repo_id: String, path: String, force: bool) -> Result<(), CommandError> {
    Ok(state.board.remove_repo_worktree(repo_id, path, force).await?)
}
