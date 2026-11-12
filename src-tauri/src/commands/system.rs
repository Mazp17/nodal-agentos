//! "Import data from a previous version…": Tauri command, thin over `System::import_legacy`.
//! The import logic (copy, then import inside a transaction) lives in `nodal_store::legacy`.

use std::sync::Arc;

use tauri::State;

use nodal_app::App;

use crate::commands::CommandError;

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn import_legacy_data(
    db: State<'_, Arc<App>>,
    folder: String,
) -> Result<nodal_store::legacy::LegacyImportReport, CommandError> {
    db.system.import_legacy(folder).await.map_err(CommandError::from)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn repo_branch(app: State<'_, Arc<App>>, repo_id: String) -> Result<Option<String>, CommandError> {
    app.system.repo_branch(repo_id).await.map_err(CommandError::from)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn list_repo_worktrees(
    app: State<'_, Arc<App>>,
    repo_id: String,
) -> Result<Vec<nodal_domain::model::RepoWorktree>, CommandError> {
    app.system.list_repo_worktrees(repo_id).await.map_err(CommandError::from)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn list_repo_dir(
    app: State<'_, Arc<App>>,
    repo_id: String,
    dir: String,
    worktree: Option<String>,
) -> Result<Vec<nodal_domain::model::DirEntry>, CommandError> {
    app.system.list_repo_dir(repo_id, dir, worktree).await.map_err(CommandError::from)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn read_repo_file(
    app: State<'_, Arc<App>>,
    repo_id: String,
    path: String,
    worktree: Option<String>,
) -> Result<nodal_domain::model::RepoFile, CommandError> {
    app.system.read_repo_file(repo_id, path, worktree).await.map_err(CommandError::from)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn repo_changes(
    app: State<'_, Arc<App>>,
    repo_id: String,
    worktree: Option<String>,
) -> Result<Vec<nodal_domain::diff::FileDiff>, CommandError> {
    app.system.repo_changes(repo_id, worktree).await.map_err(CommandError::from)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn write_repo_file(
    app: State<'_, Arc<App>>,
    repo_id: String,
    path: String,
    content: String,
    version: String,
    force: bool,
    worktree: Option<String>,
) -> Result<nodal_domain::model::RepoFile, CommandError> {
    app.system
        .write_repo_file(repo_id, path, content, version, force, worktree)
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn revert_repo_file(
    app: State<'_, Arc<App>>,
    repo_id: String,
    path: String,
    worktree: Option<String>,
) -> Result<(), CommandError> {
    app.system.revert_repo_file(repo_id, path, worktree).await.map_err(CommandError::from)
}
