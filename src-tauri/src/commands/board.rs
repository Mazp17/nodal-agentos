//! Board Tauri commands (signatures in `src/domain/api.ts`): projects, repos, tasks,
//! relations, executors and settings. Thin: parse, then `state.board.<method>(…)`.

use std::sync::Arc;

use tauri::State;

use nodal_app::App;

use nodal_domain::board::dto::*;
use nodal_domain::model::executors::ExecutorInfo;
use nodal_domain::model::*;

use crate::commands::CommandError;

// ---------- Projects ----------

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn list_projects(state: State<'_, Arc<App>>, include_archived: Option<bool>) -> Result<Vec<Project>, CommandError> {
    Ok(state.board.list_projects(include_archived).await?)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn create_project(state: State<'_, Arc<App>>, input: NewProject) -> Result<Project, CommandError> {
    Ok(state.board.create_project(input).await?)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn update_project(state: State<'_, Arc<App>>, id: String, patch: ProjectPatch) -> Result<Project, CommandError> {
    Ok(state.board.update_project(id, patch).await?)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn delete_project(state: State<'_, Arc<App>>, id: String) -> Result<(), CommandError> {
    Ok(state.delete_project(id).await?)
}

// ---------- Repos ----------

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn list_repos(state: State<'_, Arc<App>>, project_id: Option<String>) -> Result<Vec<Repo>, CommandError> {
    Ok(state.board.list_repos(project_id).await?)
}

/// `path` can be any folder inside the repo: it's resolved to the git root.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn add_repo(state: State<'_, Arc<App>>, project_id: String, input: NewRepo) -> Result<Repo, CommandError> {
    Ok(state.board.add_repo(project_id, input).await?)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn update_repo(state: State<'_, Arc<App>>, id: String, patch: RepoPatch) -> Result<Repo, CommandError> {
    Ok(state.board.update_repo(id, patch).await?)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn delete_repo(state: State<'_, Arc<App>>, id: String) -> Result<(), CommandError> {
    Ok(state.board.delete_repo(id).await?)
}

// ---------- Tasks ----------

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn list_tasks(state: State<'_, Arc<App>>, project_id: Option<String>) -> Result<Vec<Task>, CommandError> {
    Ok(state.board.list_tasks(project_id).await?)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn get_task(state: State<'_, Arc<App>>, id: String) -> Result<Task, CommandError> {
    Ok(state.board.get_task(id).await?)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn create_task(state: State<'_, Arc<App>>, input: NewTask) -> Result<Task, CommandError> {
    Ok(state.board.create_task(input).await?)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn update_task(state: State<'_, Arc<App>>, id: String, patch: TaskPatch) -> Result<Task, CommandError> {
    Ok(state.board.update_task(id, patch).await?)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn delete_task(state: State<'_, Arc<App>>, id: String) -> Result<(), CommandError> {
    Ok(state.board.delete_task(id).await?)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn move_task(state: State<'_, Arc<App>>, id: String, status: TaskStatus, position: f64) -> Result<Task, CommandError> {
    Ok(state.board.move_task(id, status, position).await?)
}

/// New order of a board column (ids from a single project, all with `status`).
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn reorder_tasks(state: State<'_, Arc<App>>, status: TaskStatus, ordered_ids: Vec<String>) -> Result<(), CommandError> {
    Ok(state.board.reorder_tasks(status, ordered_ids).await?)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn read_task_plan(state: State<'_, Arc<App>>, id: String) -> Result<String, CommandError> {
    Ok(state.board.read_task_plan(id).await?)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn list_task_relations(state: State<'_, Arc<App>>, task_id: String) -> Result<Vec<TaskRelation>, CommandError> {
    Ok(state.board.list_task_relations(task_id).await?)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn add_task_relation(
    state: State<'_, Arc<App>>,
    task_id: String,
    other_id: String,
    kind: RelationKind,
) -> Result<(), CommandError> {
    Ok(state.board.add_task_relation(task_id, other_id, kind).await?)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn remove_task_relation(
    state: State<'_, Arc<App>>,
    task_id: String,
    other_id: String,
    kind: RelationKind,
) -> Result<(), CommandError> {
    Ok(state.board.remove_task_relation(task_id, other_id, kind).await?)
}

// ---------- Executors ----------

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn list_executors(state: State<'_, Arc<App>>, repo_id: Option<String>) -> Result<Vec<ExecutorInfo>, CommandError> {
    Ok(state.board.list_executors(repo_id).await?)
}

/// Agents and workflows the project hides from its pickers. `list_executors` stays unfiltered.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn list_hidden_executors(state: State<'_, Arc<App>>, project_id: String) -> Result<Vec<HiddenExecutor>, CommandError> {
    Ok(state.board.list_hidden_executors(project_id).await?)
}

/// Hides or shows one agent or workflow in the project; returns the project's new set.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn set_executor_hidden(
    state: State<'_, Arc<App>>,
    project_id: String,
    key: HiddenExecutor,
    hidden: bool,
) -> Result<Vec<HiddenExecutor>, CommandError> {
    Ok(state.board.set_executor_hidden(project_id, key, hidden).await?)
}

// ---------- Settings ----------

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn get_settings(state: State<'_, Arc<App>>) -> Result<Settings, CommandError> {
    Ok(state.board.get_settings().await?)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn set_settings(state: State<'_, Arc<App>>, settings: Settings) -> Result<Settings, CommandError> {
    Ok(state.inner().set_settings(settings).await?)
}
