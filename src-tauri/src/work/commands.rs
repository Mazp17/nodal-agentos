//! Comandos de Tauri de F1-B (firmas en `src/domain/api.ts`). Delgados: validan ids,
//! corren `ops`/`launch` con la base y disparan la cola.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rusqlite::Connection;
use tauri::State;

use crate::db::queries::{projects, relations, repos, runs as qruns, tasks};
use crate::db::{rows, with_db, DbError};
use crate::domain::*;
use crate::events::Kind;
use crate::runs::types::Transcript;
use crate::runs::{claude_bin, claude_fs, terminal};
use crate::util::{blocking, check_id, now_ms, paths};

use super::diff::{self, RunDiff};
use super::dto::*;
use super::executors::{self, ExecutorInfo};
use super::queue::EndSignal;
use super::transitions::{RunEnd, NOTE_STOPPED};
use super::{kick, launch, ops, pump, validate, worktree, Inner, WorkState};

const OPEN_TIMEOUT: Duration = Duration::from_secs(3);
/// Lo que cambia al encolar, lanzar o cancelar un run.
const RUN_KINDS: &[Kind] = &[Kind::Runs, Kind::Queue, Kind::Tasks];

/// Corre `f` con la conexión; los errores de `ops` ya vienen listos para mostrar.
async fn db<T, F>(inner: &Arc<Inner>, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&mut Connection) -> Result<T, String> + Send + 'static,
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

// ---------- Proyectos ----------

#[tauri::command]
pub async fn list_projects(state: State<'_, WorkState>, include_archived: Option<bool>) -> Result<Vec<Project>, String> {
    let all = include_archived.unwrap_or(false);
    db(&state.0, move |c| Ok(projects::list(c, all)?)).await
}

#[tauri::command]
pub async fn create_project(state: State<'_, WorkState>, input: NewProject) -> Result<Project, String> {
    let p = db(&state.0, move |c| ops::create_project(c, &input, now_ms())).await?;
    state.0.events.notify(Kind::Projects, None);
    Ok(p)
}

#[tauri::command]
pub async fn update_project(state: State<'_, WorkState>, id: String, patch: ProjectPatch) -> Result<Project, String> {
    check_id(&id, "project")?;
    let p = db(&state.0, move |c| ops::update_project(c, &id, &patch, now_ms())).await?;
    state.0.events.notify(Kind::Projects, None);
    Ok(p)
}

#[tauri::command]
pub async fn delete_project(state: State<'_, WorkState>, id: String) -> Result<(), String> {
    check_id(&id, "project")?;
    let env = state.0.env.clone();
    let task_ids = db(&state.0, move |c| ops::delete_project(c, &id)).await?;
    state.0.events.notify_all(&[Kind::Projects, Kind::Tasks, Kind::Runs, Kind::Queue, Kind::Sources], None);
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

/// `path` puede ser cualquier carpeta dentro del repo: se resuelve a la raíz git.
#[tauri::command]
pub async fn add_repo(state: State<'_, WorkState>, project_id: String, input: NewRepo) -> Result<Repo, String> {
    check_id(&project_id, "project")?;
    let path = input.path.clone();
    let root = blocking(move || paths::require_git_root(&path)).await?;
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

// ---------- Tareas ----------

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

/// "Clean up": borra el worktree y la rama de la tarea. Rechaza con runs en curso.
#[tauri::command]
pub async fn cleanup_worktree(state: State<'_, WorkState>, task_id: String) -> Result<Task, String> {
    check_id(&task_id, "task")?;
    let id = task_id.clone();
    let (task, repo) = db(&state.0, move |c| {
        let t = tasks::get(c, &id)?;
        if !qruns::pending_for_task(c, &id)?.is_empty() {
            return Err("The task has a queued or running run: cancel it first.".into());
        }
        let r = repos::get(c, &t.repo_id)?;
        Ok((t, r))
    })
    .await?;
    let Some(wt) = task.worktree.clone() else { return Ok(task) };
    blocking(move || worktree::cleanup(Path::new(&repo.path), &wt)).await?;
    let t = db(&state.0, move |c| {
        let mut t = tasks::get(c, &task_id)?;
        t.worktree = None;
        t.updated_at = now_ms();
        tasks::update(c, &t)?;
        Ok(t)
    })
    .await?;
    state.0.events.notify(Kind::Tasks, Some(&t.project_id));
    Ok(t)
}

// ---------- Ejecutores y runs ----------

#[tauri::command]
pub async fn list_executors(state: State<'_, WorkState>, repo_id: Option<String>) -> Result<Vec<ExecutorInfo>, String> {
    let repo_id = opt_id(repo_id, "repo")?;
    let repo_path = match repo_id {
        Some(id) => Some(db(&state.0, move |c| Ok(repos::get(c, &id)?.path)).await?),
        None => None,
    };
    let claude = state.0.env.claude_dir.clone();
    blocking(move || Ok(executors::catalog(claude.as_deref(), repo_path.as_deref().map(Path::new)))).await
}

#[tauri::command]
pub async fn list_task_runs(state: State<'_, WorkState>, task_id: Option<String>) -> Result<Vec<Run>, String> {
    let task_id = opt_id(task_id, "task")?;
    db(&state.0, move |c| Ok(qruns::list(c, task_id.as_deref())?)).await
}

#[tauri::command]
pub async fn list_queue(state: State<'_, WorkState>) -> Result<Vec<Run>, String> {
    db(&state.0, |c| Ok(qruns::queue(c)?)).await
}

#[tauri::command]
pub async fn launch_task(state: State<'_, WorkState>, task_id: String, input: Option<LaunchInput>) -> Result<Run, String> {
    check_id(&task_id, "task")?;
    let env = state.0.env.clone();
    let input = input.unwrap_or_default();
    let run = db(&state.0, move |c| launch::enqueue_work(c, &env, &task_id, &input, false, now_ms())).await?;
    kick(&state.0);
    state.0.events.notify_all(RUN_KINDS, None);
    Ok(run)
}

#[tauri::command]
pub async fn hand_off(
    state: State<'_, WorkState>,
    task_id: String,
    executor: Executor,
    extra_instructions: Option<String>,
) -> Result<Run, String> {
    check_id(&task_id, "task")?;
    let env = state.0.env.clone();
    let input = LaunchInput { executor: Some(executor), extra_instructions, ..Default::default() };
    let run = db(&state.0, move |c| launch::enqueue_work(c, &env, &task_id, &input, true, now_ms())).await?;
    kick(&state.0);
    state.0.events.notify_all(RUN_KINDS, None);
    Ok(run)
}

#[tauri::command]
pub async fn review_now(state: State<'_, WorkState>, task_id: String, reviewer: Option<String>) -> Result<Run, String> {
    check_id(&task_id, "task")?;
    let env = state.0.env.clone();
    let run = db(&state.0, move |c| launch::enqueue_review(c, &env, &task_id, reviewer.as_deref(), now_ms())).await?;
    kick(&state.0);
    state.0.events.notify_all(RUN_KINDS, None);
    Ok(run)
}

/// Confirma un run migrado que quedó en cola (no se lanza solo).
#[tauri::command]
pub async fn confirm_run(state: State<'_, WorkState>, run_id: String) -> Result<Run, String> {
    check_id(&run_id, "run")?;
    let env = state.0.env.clone();
    let run = db(&state.0, move |c| launch::confirm_legacy(c, &env, &run_id, now_ms())).await?;
    kick(&state.0);
    state.0.events.notify_all(RUN_KINDS, None);
    Ok(run)
}

#[tauri::command]
pub async fn reorder_queue(state: State<'_, WorkState>, run_ids: Vec<String>) -> Result<(), String> {
    for id in &run_ids {
        check_id(id, "run")?;
    }
    db(&state.0, move |c| Ok(qruns::reorder_queue(c, &run_ids)?)).await?;
    state.0.events.notify(Kind::Queue, None);
    Ok(())
}

/// Base del diff de un run: la del worktree de la tarea si el run corrió ahí.
fn diff_base(task: Option<&Task>, run: &Run) -> Option<String> {
    let wt = task.and_then(|t| t.worktree.as_ref())?;
    (run.isolation == Some(Isolation::Worktree) && Path::new(&wt.path) == Path::new(&run.cwd)).then(|| wt.base.clone())
}

/// Saca de la cola un run `queued`, o detiene uno lanzado: guarda lo que dejó a medias como
/// patch en `<app_data>/runs/<id>/stopped.patch` y la tarea pasa a Blocked.
#[tauri::command]
pub async fn cancel_run(state: State<'_, WorkState>, run_id: String) -> Result<Run, String> {
    let r = cancel_run_inner(&state.0, run_id).await;
    state.0.events.notify_all(RUN_KINDS, None);
    r
}

async fn cancel_run_inner(inner: &Arc<Inner>, run_id: String) -> Result<Run, String> {
    check_id(&run_id, "run")?;
    let inner = inner.clone();
    // Con el turno de la cola: la pasada no puede cerrar este run en el medio (quedaría
    // `finished`, sin la nota del patch, y hasta con el revisor encolado).
    let _turn = inner.pump.lock().await;
    let id = run_id.clone();
    let (run, task, repo_path) = db(&inner, move |c| {
        let r = qruns::get(c, &id)?;
        let t = match &r.task_id {
            Some(t) => rows::get_task(c, t)?,
            None => None,
        };
        let repo_path = match &r.repo_id {
            Some(id) => rows::get_repo(c, id)?.map(|r| r.path),
            None => None,
        };
        Ok((r, t, repo_path))
    })
    .await?;
    match run.status {
        RunStatus::Queued => {
            let id = run_id.clone();
            db(&inner, move |c| {
                let now = now_ms();
                let tx = c.transaction().map_err(|e| e.to_string())?;
                if !qruns::transition(&tx, &id, RunStatus::Queued, RunStatus::Canceled)? {
                    return Err("The run is no longer queued.".into());
                }
                tx.execute("UPDATE runs SET finished_at = ?2 WHERE id = ?1", rusqlite::params![id, now])
                    .map_err(|e| e.to_string())?;
                // Sin otros runs pendientes, la tarea no queda en In Progress: vuelve a Todo si
                // era trabajo, o a Blocked si era el revisor (el gate no pasó).
                if let Some(t) = task {
                    if t.status == TaskStatus::InProgress && qruns::pending_for_task(&tx, &t.id)?.is_empty() {
                        let next = match run.kind {
                            RunKind::Work => TaskStatus::Todo,
                            RunKind::Review => TaskStatus::Blocked,
                        };
                        ops::apply_task_transition(&tx, &t, Some(next), None, None, now)?;
                    }
                }
                tx.commit().map_err(|e| e.to_string())?;
                Ok(qruns::get(c, &id)?)
            })
            .await
        }
        RunStatus::Launching => Err("The run is launching: wait for it to start and stop it then.".into()),
        RunStatus::Launched => {
            let claude_id = run.claude_run_id.clone().ok_or("The run has no session id yet.")?;
            // Lo que haya a medias, antes de detenerlo (los workflows trabajan en su worktree).
            let patch_path = if matches!(run.executor, Executor::Workflow { .. }) {
                None
            } else {
                let (cwd, base) = (run.cwd.clone(), diff_base(task.as_ref(), &run));
                let target = inner.env.stopped_patch_path(&run.id);
                blocking(move || {
                    let Ok((patch, _)) = diff::collect(Path::new(&cwd), base.as_deref()) else { return Ok(None) };
                    if patch.trim().is_empty() {
                        return Ok(None);
                    }
                    crate::util::write_atomic(&target, patch.as_bytes())?;
                    Ok(Some(target))
                })
                .await
                .unwrap_or_else(|e| {
                    eprintln!("work: couldn't save the partial patch: {e}");
                    None
                })
            };
            terminal::stop(&claude_id).await?;
            let tokens = match (&run.executor, run.session_id.clone()) {
                (Executor::Workflow { .. }, _) | (_, None) => None,
                (_, Some(sid)) => {
                    let cwd = run.cwd.clone();
                    blocking(move || Ok(crate::runs::session_tokens(&sid, &cwd))).await.unwrap_or(None)
                }
            };
            let note = match &patch_path {
                Some(p) => format!("{NOTE_STOPPED} Stopped by the user; partial changes saved to {}.", p.display()),
                None => format!("{NOTE_STOPPED} Stopped by the user."),
            };
            let end = RunEnd {
                signal: EndSignal::Stopped,
                outcome: RunOutcome::Stopped,
                summary: None,
                pr: None,
                branch: None,
                verdict: None,
                note: Some(note),
                missing_report: false,
                tokens,
            };
            let env = inner.env.clone();
            let id = run.id.clone();
            let (env2, executor) = (env.clone(), run.executor.clone());
            let (_, manages) = blocking(move || Ok(pump::workflow_meta(&env2, repo_path.as_deref(), &executor))).await?;
            db(&inner, move |c| {
                pump::apply_end(c, &env, &run, &end, RunStatus::Canceled, manages.as_deref(), now_ms())?;
                Ok(qruns::get(c, &id)?)
            })
            .await
        }
        _ => Err("The run already finished.".into()),
    }
}

/// Transcript de un run de agente, Claude o revisor (la sesión principal). Los workflows
/// tienen uno por agente: `get_agent_transcript`. `None` si la sesión todavía no tiene archivo.
/// `limit`: items más recientes (default 200, máx. 2000).
#[tauri::command]
pub async fn get_run_transcript(
    state: State<'_, WorkState>,
    run_id: String,
    limit: Option<u32>,
) -> Result<Option<Transcript>, String> {
    check_id(&run_id, "run")?;
    let run = db(&state.0, move |c| Ok(qruns::get(c, &run_id)?)).await?;
    let label = match &run.executor {
        Executor::Workflow { .. } => {
            return Err("Workflow runs have one transcript per agent: open it from the run detail.".into())
        }
        Executor::Agent { name, .. } => name.clone(),
        Executor::Claude => "Claude".into(),
    };
    let Some(sid) = run.session_id.clone().filter(|s| claude_fs::is_valid_session_id(s)) else { return Ok(None) };
    let Some(claude_dir) = state.0.env.claude_dir.clone() else {
        return Err("Couldn't locate the Claude Code folder ($HOME is not set).".into());
    };
    let limit = limit.unwrap_or(claude_fs::TRANSCRIPT_DEFAULT_LIMIT).clamp(1, claude_fs::TRANSCRIPT_MAX_LIMIT) as usize;
    blocking(move || {
        let Some(path) = claude_fs::find_session_jsonl(&claude_dir.join("projects"), &run.cwd, &sid) else {
            return Ok(None);
        };
        claude_fs::read_session_transcript(&path, &run.id, Some(label), run.options.model.clone(), limit)
    })
    .await
}

#[tauri::command]
pub async fn run_diff(state: State<'_, WorkState>, run_id: String) -> Result<RunDiff, String> {
    check_id(&run_id, "run")?;
    let (run, task, repo) = db(&state.0, move |c| {
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
    blocking(move || {
        // Un workflow trabaja en su propio worktree: se mira su rama desde el repo.
        if let (Executor::Workflow { .. }, Some(branch), Some(repo)) = (&run.executor, &run.branch, &repo) {
            let repo_dir = Path::new(&repo.path);
            let base = worktree::current_base(repo_dir)?;
            let patch = diff::collect_branch(repo_dir, &base, branch)?;
            let commits = diff::commits(repo_dir, &base, branch).unwrap_or_default();
            return Ok(RunDiff {
                branch: Some(branch.clone()),
                commits,
                live,
                base,
                cwd: repo.path.clone(),
                includes_working_tree: false,
                files: diff::parse(&patch),
                patch,
            });
        }
        let cwd = Path::new(&run.cwd);
        if !cwd.is_dir() {
            return Err(format!("The run's folder no longer exists: {}", run.cwd));
        }
        let base = diff_base(task.as_ref(), &run);
        let (patch, dirty) = diff::collect(cwd, base.as_deref())?;
        let commits = match &base {
            Some(b) => diff::commits(cwd, b, "HEAD").unwrap_or_default(),
            None => Vec::new(),
        };
        Ok(RunDiff {
            branch: diff::current_branch(cwd),
            commits,
            live,
            base: base.unwrap_or_else(|| "HEAD".into()),
            cwd: run.cwd.clone(),
            includes_working_tree: dirty,
            files: diff::parse(&patch),
            patch,
        })
    })
    .await
}

async fn run_cwd(inner: &Arc<Inner>, run_id: String) -> Result<PathBuf, String> {
    check_id(&run_id, "run")?;
    let cwd = db(inner, move |c| Ok(qruns::get(c, &run_id)?.cwd)).await?;
    let p = PathBuf::from(&cwd);
    if !p.is_dir() {
        return Err(format!("The run's folder no longer exists: {cwd}"));
    }
    Ok(p)
}

/// Lanza el proceso y espera un poco: si sale con error enseguida se informa; si sigue
/// corriendo (algunos CLIs de editores no vuelven), se lo deja y se cosecha en segundo plano.
async fn spawn_open(mut cmd: tokio::process::Command, what: &str) -> Result<(), String> {
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("Couldn't run {what}: {e}"))?;
    match tokio::time::timeout(OPEN_TIMEOUT, child.wait()).await {
        Ok(Ok(s)) if s.success() => Ok(()),
        Ok(Ok(s)) => {
            let mut err = String::new();
            if let Some(mut e) = child.stderr.take() {
                use tokio::io::AsyncReadExt;
                let _ = e.read_to_string(&mut err).await;
            }
            Err(format!("{what} failed ({s}): {}", err.trim()))
        }
        Ok(Err(e)) => Err(format!("{what} failed: {e}")),
        Err(_) => {
            tauri::async_runtime::spawn(async move {
                let _ = child.wait().await;
            });
            Ok(())
        }
    }
}

/// Abre la carpeta del run en Finder.
#[tauri::command]
pub async fn open_worktree(state: State<'_, WorkState>, run_id: String) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("Opening Finder is only available on macOS.".into());
    }
    let dir = run_cwd(&state.0, run_id).await?;
    let mut cmd = tokio::process::Command::new("/usr/bin/open");
    cmd.arg("--").arg(dir);
    spawn_open(cmd, "open").await
}

/// Abre la carpeta del run (o `file` dentro de ella) en el editor de Settings; sin editor,
/// con la app por defecto del sistema.
#[tauri::command]
pub async fn open_in_editor(state: State<'_, WorkState>, run_id: String, file: Option<String>) -> Result<(), String> {
    let dir = run_cwd(&state.0, run_id).await?;
    let target = match file.as_deref().map(str::trim).filter(|f| !f.is_empty()) {
        Some(f) => {
            let (dir, f) = (dir.clone(), f.to_string());
            Some(
                blocking(move || {
                    let canon = dir.join(&f).canonicalize().map_err(|_| format!("The file doesn't exist: {f}"))?;
                    let root = dir.canonicalize().map_err(|e| e.to_string())?;
                    if !canon.starts_with(&root) {
                        return Err("The file must be inside the run's folder.".into());
                    }
                    Ok(canon)
                })
                .await?,
            )
        }
        None => None,
    };
    let editor = db(&state.0, |c| Ok(rows::load_settings(c)?.editor)).await?;
    let mut cmd = match editor {
        Some(e) => {
            let e = validate::editor(&e)?;
            let bin = claude_bin::resolve_bin(&e).ok_or_else(|| format!("Couldn't find `{e}` in PATH."))?;
            let mut cmd = tokio::process::Command::new(bin);
            cmd.env("PATH", claude_bin::augmented_path());
            cmd.arg(&dir);
            cmd
        }
        None => {
            if !cfg!(target_os = "macos") {
                return Err("Set an editor in Settings.".into());
            }
            // `-t`: siempre como texto. Sin eso, un `.command` o `.app` que dejó el agente
            // se ejecutaría. La carpeta se abre en Finder.
            let mut cmd = tokio::process::Command::new("/usr/bin/open");
            if target.is_some() {
                cmd.arg("-t");
            }
            cmd.arg("--");
            if target.is_none() {
                cmd.arg(&dir);
            }
            cmd
        }
    };
    if let Some(t) = target {
        cmd.arg(t);
    }
    spawn_open(cmd, "the editor").await
}

// ---------- Settings ----------

#[tauri::command]
pub async fn get_settings(state: State<'_, WorkState>) -> Result<Settings, String> {
    db(&state.0, |c| Ok(rows::load_settings(c)?)).await
}

#[tauri::command]
pub async fn set_settings(state: State<'_, WorkState>, settings: Settings) -> Result<Settings, String> {
    let s = db(&state.0, move |c| ops::set_settings(c, &settings)).await?;
    state.0.events.notify(Kind::Projects, None);
    // Más concurrencia puede liberar lugar para lo encolado.
    kick(&state.0);
    Ok(s)
}
