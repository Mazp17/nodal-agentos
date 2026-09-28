//! Board Tauri commands (signatures in `src/domain/api.ts`): projects, repos, tasks,
//! relations, executors and settings. Thin: they validate ids, run `ops` against the
//! database and kick the queue.

use tauri::State;

use crate::db::queries::{hidden_executors, projects, relations, repos, tasks};
use crate::db::{with_db, DbError};
use crate::domain::*;
use crate::work::dto::*;
use crate::events::Kind;
use crate::util::{blocking, check_id, now_ms};
use crate::work::executors::{self, ExecutorInfo};
use crate::work::{ops, Inner, WorkState};

/// Runs `f` with the connection; `ops` errors already come ready to display.
async fn db<T, F>(inner: &std::sync::Arc<Inner>, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&mut crate::db::Connection) -> Result<T, String> + Send + 'static,
{
    Ok(with_db(&inner.db, move |c| f(c).map_err(DbError::Invalid)).await?)
}

fn opt_id(id: Option<String>, what: &str) -> Result<Option<String>, String> {
    let id = id.filter(|s| !s.trim().is_empty());
    if let Some(i) = &id {
        check_id(i, what)?;
    }
    Ok(id)
}

// ---------- Projects ----------

#[tauri::command]
pub async fn list_projects(state: State<'_, WorkState>, include_archived: Option<bool>) -> Result<Vec<Project>, String> {
    let all = include_archived.unwrap_or(false);
    db(&state.0, move |c| Ok(projects::list(c, all)?)).await
}

#[tauri::command]
pub async fn create_project(state: State<'_, WorkState>, input: NewProject) -> Result<Project, String> {
    let env = state.0.env.clone();
    let p = db(&state.0, move |c| ops::create_project(c, &env, &input, now_ms())).await?;
    state.0.events.notify(Kind::Projects, None);
    Ok(p)
}

#[tauri::command]
pub async fn update_project(state: State<'_, WorkState>, id: String, patch: ProjectPatch) -> Result<Project, String> {
    check_id(&id, "project")?;
    let env = state.0.env.clone();
    let p = db(&state.0, move |c| ops::update_project(c, &env, &id, &patch, now_ms())).await?;
    state.0.events.notify(Kind::Projects, None);
    Ok(p)
}

#[tauri::command]
pub async fn delete_project(
    state: State<'_, WorkState>,
    chats: State<'_, crate::chats::ChatState>,
    id: String,
) -> Result<(), String> {
    check_id(&id, "project")?;
    let env = state.0.env.clone();
    chats.0.stop_project(&id);
    let task_ids = db(&state.0, move |c| ops::delete_project(c, &id)).await?;
    state.0.events.notify_all(&[Kind::Projects, Kind::Tasks, Kind::Runs, Kind::Queue, Kind::Sources, Kind::Chats], None);
    blocking(move || {
        for t in task_ids {
            let _ = std::fs::remove_dir_all(env.plan_dir(&t));
        }
        Ok(())
    })
    .await
}

// ---------- Repos ----------

#[tauri::command]
pub async fn list_repos(state: State<'_, WorkState>, project_id: Option<String>) -> Result<Vec<Repo>, String> {
    let project_id = opt_id(project_id, "project")?;
    db(&state.0, move |c| Ok(repos::list(c, project_id.as_deref())?)).await
}

/// `path` can be any folder inside the repo: it's resolved to the git root.
#[tauri::command]
pub async fn add_repo(state: State<'_, WorkState>, project_id: String, input: NewRepo) -> Result<Repo, String> {
    check_id(&project_id, "project")?;
    let path = input.path.clone();
    let root = blocking(move || crate::util::paths::require_git_root(&path)).await?;
    let r = db(&state.0, move |c| ops::add_repo(c, &project_id, &input, &root, now_ms())).await?;
    state.0.events.notify(Kind::Projects, Some(&r.project_id));
    Ok(r)
}

#[tauri::command]
pub async fn update_repo(state: State<'_, WorkState>, id: String, patch: RepoPatch) -> Result<Repo, String> {
    check_id(&id, "repo")?;
    let r = db(&state.0, move |c| ops::update_repo(c, &id, &patch)).await?;
    state.0.events.notify(Kind::Projects, Some(&r.project_id));
    Ok(r)
}

#[tauri::command]
pub async fn delete_repo(state: State<'_, WorkState>, id: String) -> Result<(), String> {
    check_id(&id, "repo")?;
    db(&state.0, move |c| ops::delete_repo(c, &id)).await?;
    state.0.events.notify_all(&[Kind::Projects, Kind::Tasks], None);
    Ok(())
}

// ---------- Tasks ----------

#[tauri::command]
pub async fn list_tasks(state: State<'_, WorkState>, project_id: Option<String>) -> Result<Vec<Task>, String> {
    let project_id = opt_id(project_id, "project")?;
    db(&state.0, move |c| Ok(tasks::list(c, project_id.as_deref())?)).await
}

#[tauri::command]
pub async fn get_task(state: State<'_, WorkState>, id: String) -> Result<Task, String> {
    check_id(&id, "task")?;
    db(&state.0, move |c| Ok(tasks::get(c, &id)?)).await
}

#[tauri::command]
pub async fn create_task(state: State<'_, WorkState>, input: NewTask) -> Result<Task, String> {
    check_id(&input.project_id, "project")?;
    check_id(&input.repo_id, "repo")?;
    let env = state.0.env.clone();
    let t = db(&state.0, move |c| ops::create_task(c, &env, &input, now_ms())).await?;
    state.0.events.notify(Kind::Tasks, Some(&t.project_id));
    Ok(t)
}

#[tauri::command]
pub async fn update_task(state: State<'_, WorkState>, id: String, patch: TaskPatch) -> Result<Task, String> {
    check_id(&id, "task")?;
    if let Some(r) = &patch.repo_id {
        check_id(r, "repo")?;
    }
    let env = state.0.env.clone();
    let t = db(&state.0, move |c| ops::update_task(c, &env, &id, &patch, now_ms())).await?;
    state.0.events.notify(Kind::Tasks, Some(&t.project_id));
    Ok(t)
}

#[tauri::command]
pub async fn delete_task(state: State<'_, WorkState>, id: String) -> Result<(), String> {
    check_id(&id, "task")?;
    let env = state.0.env.clone();
    db(&state.0, move |c| ops::delete_task(c, &env, &id)).await?;
    state.0.events.notify_all(&[Kind::Tasks, Kind::Runs, Kind::Queue], None);
    Ok(())
}

#[tauri::command]
pub async fn move_task(state: State<'_, WorkState>, id: String, status: TaskStatus, position: f64) -> Result<Task, String> {
    check_id(&id, "task")?;
    let t = db(&state.0, move |c| ops::move_task(c, &id, status, position, now_ms())).await?;
    state.0.events.notify(Kind::Tasks, Some(&t.project_id));
    Ok(t)
}

/// New order of a board column (ids from a single project, all with `status`).
#[tauri::command]
pub async fn reorder_tasks(state: State<'_, WorkState>, status: TaskStatus, ordered_ids: Vec<String>) -> Result<(), String> {
    for id in &ordered_ids {
        check_id(id, "task")?;
    }
    let project = db(&state.0, move |c| ops::reorder_tasks(c, status, &ordered_ids, now_ms())).await?;
    if let Some(p) = project {
        state.0.events.notify(Kind::Tasks, Some(&p));
    }
    Ok(())
}

#[tauri::command]
pub async fn read_task_plan(state: State<'_, WorkState>, id: String) -> Result<String, String> {
    check_id(&id, "task")?;
    let env = state.0.env.clone();
    db(&state.0, move |c| ops::read_task_plan(c, &env, &id)).await
}

#[tauri::command]
pub async fn list_task_relations(state: State<'_, WorkState>, task_id: String) -> Result<Vec<TaskRelation>, String> {
    check_id(&task_id, "task")?;
    db(&state.0, move |c| Ok(relations::list(c, &task_id)?)).await
}

#[tauri::command]
pub async fn add_task_relation(
    state: State<'_, WorkState>,
    task_id: String,
    other_id: String,
    kind: RelationKind,
) -> Result<(), String> {
    check_id(&task_id, "task")?;
    check_id(&other_id, "task")?;
    db(&state.0, move |c| ops::add_relation(c, &task_id, &other_id, kind)).await?;
    state.0.events.notify(Kind::Tasks, None);
    Ok(())
}

#[tauri::command]
pub async fn remove_task_relation(
    state: State<'_, WorkState>,
    task_id: String,
    other_id: String,
    kind: RelationKind,
) -> Result<(), String> {
    check_id(&task_id, "task")?;
    check_id(&other_id, "task")?;
    db(&state.0, move |c| ops::remove_relation(c, &task_id, &other_id, kind)).await?;
    state.0.events.notify(Kind::Tasks, None);
    Ok(())
}

// ---------- Executors ----------

#[tauri::command]
pub async fn list_executors(state: State<'_, WorkState>, repo_id: Option<String>) -> Result<Vec<ExecutorInfo>, String> {
    let repo_id = opt_id(repo_id, "repo")?;
    let repo_path = match repo_id {
        Some(id) => Some(db(&state.0, move |c| Ok(repos::get(c, &id)?.path)).await?),
        None => None,
    };
    let claude = state.0.env.claude_dir.clone();
    blocking(move || Ok(executors::catalog(claude.as_deref(), repo_path.as_deref().map(std::path::Path::new)))).await
}

/// Agents and workflows the project hides from its pickers. `list_executors` stays unfiltered.
#[tauri::command]
pub async fn list_hidden_executors(state: State<'_, WorkState>, project_id: String) -> Result<Vec<HiddenExecutor>, String> {
    check_id(&project_id, "project")?;
    db(&state.0, move |c| Ok(hidden_executors::list(c, &project_id)?)).await
}

/// Hides or shows one agent or workflow in the project; returns the project's new set.
#[tauri::command]
pub async fn set_executor_hidden(
    state: State<'_, WorkState>,
    project_id: String,
    key: HiddenExecutor,
    hidden: bool,
) -> Result<Vec<HiddenExecutor>, String> {
    check_id(&project_id, "project")?;
    if let Some(r) = &key.repo_id {
        check_id(r, "repo")?;
    }
    let pid = project_id.clone();
    let out = db(&state.0, move |c| {
        hidden_executors::set(c, &pid, &key, hidden)?;
        Ok(hidden_executors::list(c, &pid)?)
    })
    .await?;
    state.0.events.notify(Kind::Projects, Some(&project_id));
    Ok(out)
}

// ---------- Settings ----------

#[tauri::command]
pub async fn get_settings(state: State<'_, WorkState>) -> Result<Settings, String> {
    db(&state.0, |c| Ok(crate::db::rows::load_settings(c)?)).await
}

#[tauri::command]
pub async fn set_settings(state: State<'_, WorkState>, settings: Settings) -> Result<Settings, String> {
    let s = db(&state.0, move |c| ops::set_settings(c, &settings)).await?;
    state.0.events.notify(Kind::Projects, None);
    // More concurrency may free up room for queued runs.
    crate::work::kick(&state.0);
    Ok(s)
}
