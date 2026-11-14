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
pub async fn list_repo_files(
    app: State<'_, Arc<App>>,
    repo_id: String,
    worktree: Option<String>,
) -> Result<Vec<String>, CommandError> {
    app.system.list_repo_files(repo_id, worktree).await.map_err(CommandError::from)
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
pub async fn repo_commits(
    app: State<'_, Arc<App>>,
    repo_id: String,
    worktree: Option<String>,
) -> Result<Vec<nodal_domain::diff::CommitInfo>, CommandError> {
    app.system.repo_commits(repo_id, worktree).await.map_err(CommandError::from)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn repo_commit(
    app: State<'_, Arc<App>>,
    repo_id: String,
    worktree: Option<String>,
    message: String,
    paths: Option<Vec<String>>,
) -> Result<nodal_domain::diff::CommitInfo, CommandError> {
    app.system.repo_commit(repo_id, worktree, message, paths.unwrap_or_default()).await.map_err(CommandError::from)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn repo_open_pr(
    app: State<'_, Arc<App>>,
    repo_id: String,
    worktree: Option<String>,
    title: String,
    body: String,
    chat_id: Option<String>,
) -> Result<String, CommandError> {
    app.system.repo_open_pr(repo_id, worktree, title, body, chat_id).await.map_err(CommandError::from)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn repo_commit_diff(
    app: State<'_, Arc<App>>,
    repo_id: String,
    worktree: Option<String>,
    sha: String,
) -> Result<Vec<nodal_domain::diff::FileDiff>, CommandError> {
    app.system.repo_commit_diff(repo_id, worktree, sha).await.map_err(CommandError::from)
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

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn repo_pr(
    app: State<'_, Arc<App>>,
    repo_id: String,
    worktree: Option<String>,
    force: Option<bool>,
) -> Result<Option<nodal_domain::model::BranchPr>, CommandError> {
    app.system.repo_pr(repo_id, worktree, force).await.map_err(CommandError::from)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn repo_link_pr(
    app: State<'_, Arc<App>>,
    repo_id: String,
    worktree: Option<String>,
    pr: String,
    chat_id: Option<String>,
) -> Result<nodal_domain::model::BranchPr, CommandError> {
    app.system.repo_link_pr(repo_id, worktree, pr, chat_id).await.map_err(CommandError::from)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn repo_unlink_pr(app: State<'_, Arc<App>>, repo_id: String, worktree: Option<String>) -> Result<(), CommandError> {
    app.system.repo_unlink_pr(repo_id, worktree).await.map_err(CommandError::from)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn repo_pr_details(
    app: State<'_, Arc<App>>,
    repo_id: String,
    worktree: Option<String>,
    number: i64,
) -> Result<serde_json::Value, CommandError> {
    app.system.repo_pr_details(repo_id, worktree, number).await.map_err(CommandError::from)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn repo_check_log(
    app: State<'_, Arc<App>>,
    repo_id: String,
    worktree: Option<String>,
    run_id: String,
) -> Result<String, CommandError> {
    app.system.repo_check_log(repo_id, worktree, run_id).await.map_err(CommandError::from)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn repo_merge_pr(
    app: State<'_, Arc<App>>,
    repo_id: String,
    worktree: Option<String>,
    number: i64,
    method: String,
) -> Result<(), CommandError> {
    app.system.repo_merge_pr(repo_id, worktree, number, method).await.map_err(CommandError::from)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn chat_prs(app: State<'_, Arc<App>>, chat_id: String) -> Result<Vec<nodal_domain::model::ChatPr>, CommandError> {
    app.system.chat_prs(chat_id).await.map_err(CommandError::from)
}
