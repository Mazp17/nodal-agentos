//! F1-B Tauri commands (signatures in `src/domain/api.ts`). Thin: they validate ids, run
//! `ops`/`launch` against the database and kick the queue.

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
/// What changes when a run is enqueued, launched or cancelled.
const RUN_KINDS: &[Kind] = &[Kind::Runs, Kind::Queue, Kind::Tasks];

/// Runs `f` with the connection; `ops` errors already come ready to display.
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

// ---------- Projects ----------

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

/// `path` can be any folder inside the repo: it's resolved to the git root.
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

#[tauri::command]
pub async fn worktree_status(state: State<'_, WorkState>, task_id: String) -> Result<worktree::WorktreeStatus, String> {
    check_id(&task_id, "task")?;
    let (task, repo) = db(&state.0, move |c| {
        let t = tasks::get(c, &task_id)?;
        let r = repos::get(c, &t.repo_id)?;
        Ok((t, r))
    })
    .await?;
    blocking(move || worktree::status(Path::new(&repo.path), task.worktree.as_ref())).await
}

/// "Clean up": deletes the task's worktree and branch. Refuses with runs in progress and,
/// without `force`, if there are unpushed commits or uncommitted changes.
///
/// The task stays marked in `cleaning` (in the same section that checks for pending runs,
/// while holding the database) until the end: meanwhile `enqueue_work` rejects it, so a
/// launch doesn't reuse the worktree being deleted.
#[tauri::command]
pub async fn cleanup_worktree(state: State<'_, WorkState>, task_id: String, force: Option<bool>) -> Result<Task, String> {
    check_id(&task_id, "task")?;
    let force = force.unwrap_or(false);
    let (id, cleaning) = (task_id.clone(), state.0.cleaning.clone());
    let (task, repo, _guard) = db(&state.0, move |c| {
        let t = tasks::get(c, &id)?;
        let guard = cleaning.mark(&id).ok_or("The task's worktree is already being cleaned up.")?;
        if !qruns::pending_for_task(c, &id)?.is_empty() {
            return Err(PENDING_ERR.into());
        }
        let r = repos::get(c, &t.repo_id)?;
        Ok((t, r, guard))
    })
    .await?;
    let Some(wt) = task.worktree.clone() else { return Ok(task) };
    let (dbh, id) = (state.0.db.clone(), task_id.clone());
    blocking(move || {
        let repo = Path::new(&repo.path);
        if !force {
            if let Some(why) = worktree::cleanup_blocker(&worktree::status(repo, Some(&wt))?) {
                return Err(why);
            }
        }
        // With the mark set no new run can appear; checked again just in case.
        if !qruns::pending_for_task(&launch::lock(&dbh), &id)?.is_empty() {
            return Err(PENDING_ERR.into());
        }
        worktree::cleanup(repo, &wt)
    })
    .await?;
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

const PENDING_ERR: &str = "The task has a queued or running run: cancel it first.";

// ---------- Executors and runs ----------

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

/// `project_id` (optional) filters by project in addition to task.
#[tauri::command]
pub async fn list_task_runs(
    state: State<'_, WorkState>,
    task_id: Option<String>,
    project_id: Option<String>,
) -> Result<Vec<Run>, String> {
    let task_id = opt_id(task_id, "task")?;
    let project_id = opt_id(project_id, "project")?;
    db(&state.0, move |c| Ok(qruns::list_filtered(c, project_id.as_deref(), task_id.as_deref())?)).await
}

/// Like `list_task_runs`, without `prompt` or `extraInstructions`.
#[tauri::command]
pub async fn list_runs_light(
    state: State<'_, WorkState>,
    project_id: Option<String>,
    task_id: Option<String>,
) -> Result<Vec<RunLight>, String> {
    let task_id = opt_id(task_id, "task")?;
    let project_id = opt_id(project_id, "project")?;
    let runs = db(&state.0, move |c| Ok(qruns::list_filtered(c, project_id.as_deref(), task_id.as_deref())?)).await?;
    Ok(runs.into_iter().map(RunLight::from).collect())
}

#[tauri::command]
pub async fn get_run(state: State<'_, WorkState>, run_id: String) -> Result<Run, String> {
    check_id(&run_id, "run")?;
    db(&state.0, move |c| Ok(qruns::get(c, &run_id)?)).await
}

/// The last run of each task (no history limit), lightweight.
#[tauri::command]
pub async fn latest_runs_by_task(state: State<'_, WorkState>, project_id: Option<String>) -> Result<Vec<RunLight>, String> {
    let project_id = opt_id(project_id, "project")?;
    let runs = db(&state.0, move |c| Ok(qruns::latest_by_task(c, project_id.as_deref())?)).await?;
    Ok(runs.into_iter().map(RunLight::from).collect())
}

/// Occupied slots/capacity, queue and what's waiting on the user (`project_id` null:
/// everything, including foreign sessions). If `claude agents` fails, it's computed without
/// the live sessions.
#[tauri::command]
pub async fn work_summary(state: State<'_, WorkState>, project_id: Option<String>) -> Result<super::queue::WorkSummary, String> {
    let project_id = opt_id(project_id, "project")?;
    let global = project_id.is_none();
    let (runs, blocked, settings) = db(&state.0, move |c| {
        let p = project_id.as_deref();
        // Tasks that changed project in the provider are also waiting on the user.
        let mut need = tasks::blocked_ids(c, p)?;
        need.extend(crate::providers::store::moved_ids(c, p)?);
        Ok((qruns::pending_of(c, p)?, need, rows::load_settings(c)?))
    })
    .await?;
    let live = crate::runs::list_runs().await.unwrap_or_else(|e| {
        eprintln!("work_summary: {e}");
        Vec::new()
    });
    let mut summary = super::queue::work_summary(&runs, &blocked, &live, settings.concurrency, global, now_ms());
    summary.pump_error = state.0.pump_error.lock().unwrap_or_else(|p| p.into_inner()).clone();
    Ok(summary)
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
    let (dbh, cleaning) = (state.0.db.clone(), state.0.cleaning.clone());
    let run = blocking(move || launch::enqueue_work(&dbh, &env, &cleaning, &task_id, &input, false, now_ms())).await?;
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
    let (dbh, cleaning) = (state.0.db.clone(), state.0.cleaning.clone());
    let run = blocking(move || launch::enqueue_work(&dbh, &env, &cleaning, &task_id, &input, true, now_ms())).await?;
    kick(&state.0);
    state.0.events.notify_all(RUN_KINDS, None);
    Ok(run)
}

#[tauri::command]
pub async fn review_now(state: State<'_, WorkState>, task_id: String, reviewer: Option<String>) -> Result<Run, String> {
    check_id(&task_id, "task")?;
    let env = state.0.env.clone();
    let (dbh, cleaning) = (state.0.db.clone(), state.0.cleaning.clone());
    let run = blocking(move || launch::enqueue_review(&dbh, &env, &cleaning, &task_id, reviewer.as_deref(), now_ms())).await?;
    kick(&state.0);
    state.0.events.notify_all(RUN_KINDS, None);
    Ok(run)
}

/// Confirms a migrated run that was left queued (it doesn't launch on its own).
#[tauri::command]
pub async fn confirm_run(state: State<'_, WorkState>, run_id: String) -> Result<Run, String> {
    check_id(&run_id, "run")?;
    let env = state.0.env.clone();
    let (dbh, cleaning) = (state.0.db.clone(), state.0.cleaning.clone());
    let run = blocking(move || launch::confirm_legacy(&dbh, &env, &cleaning, &run_id, now_ms())).await?;
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

/// Diff base of a run: the task worktree's base if the run ran there.
fn diff_base(task: Option<&Task>, run: &Run) -> Option<String> {
    let wt = task.and_then(|t| t.worktree.as_ref())?;
    (run.isolation == Some(Isolation::Worktree) && Path::new(&wt.path) == Path::new(&run.cwd)).then(|| wt.base.clone())
}

/// Dequeues a `queued` run, or stops a launched one: saves whatever it left half-done as a
/// patch in `<app_data>/runs/<id>/stopped.patch` and the task moves to Blocked.
#[tauri::command]
pub async fn cancel_run(state: State<'_, WorkState>, run_id: String) -> Result<Run, String> {
    let r = cancel_run_inner(&state.0, run_id).await;
    state.0.events.notify_all(RUN_KINDS, None);
    r
}

async fn cancel_run_inner(inner: &Arc<Inner>, run_id: String) -> Result<Run, String> {
    check_id(&run_id, "run")?;
    let inner = inner.clone();
    // Holding the queue's turn: the pass can't close this run midway (it would end up
    // `finished`, without the patch note, and even with the reviewer enqueued).
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
                // With no other pending runs, the task doesn't stay In Progress: back to Todo if
                // it was work, or to Blocked if it was the reviewer (the gate didn't pass).
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
            // Whatever is half-done, before stopping it (workflows work in their own worktree).
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
            let dbh = inner.db.clone();
            blocking(move || {
                pump::apply_end(&dbh, &env, &run, &end, RunStatus::Canceled, manages.as_deref(), now_ms())?;
                Ok(qruns::get(&launch::lock(&dbh), &id)?)
            })
            .await
        }
        _ => Err("The run already finished.".into()),
    }
}

/// Transcript of an agent, Claude or reviewer run (the main session). Workflows have one per
/// agent: `get_agent_transcript`. `None` if the session has no file yet.
/// `limit`: most recent items (default 200, max 2000).
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
        // A workflow works in its own worktree: its branch is inspected from the repo.
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

/// Spawns the process and waits a bit: if it exits with an error right away, that's
/// reported; if it keeps running (some editor CLIs don't return), it's left alone and reaped
/// in the background.
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

/// Opens the run's folder in Finder.
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

/// Opens the run's folder (or `file` inside it) in the Settings editor. Without one set,
/// uses the first known editor whose CLI is installed, and only then the system text editor.
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
    // (binary, name for errors)
    let editor = match db(&state.0, |c| Ok(rows::load_settings(c)?.editor)).await? {
        Some(e) => {
            let e = validate::editor(&e)?;
            let bin = claude_bin::resolve_bin(&e).ok_or_else(|| format!("Couldn't find `{e}` in PATH."))?;
            Some((bin, format!("`{e}`")))
        }
        None => detect_editor().map(|(e, bin)| (bin, format!("`{e}` (picked automatically; choose one in Settings)"))),
    };
    let what = editor.as_ref().map_or_else(|| "the system text editor".to_string(), |(_, w)| w.clone());
    let mut cmd = match editor {
        Some((bin, _)) => {
            let mut cmd = tokio::process::Command::new(bin);
            cmd.env("PATH", claude_bin::augmented_path());
            cmd.arg(&dir);
            cmd
        }
        None => {
            if !cfg!(target_os = "macos") {
                return Err("Set an editor in Settings.".into());
            }
            // `-t`: always as text. Without it, a `.command` or `.app` left by the agent would
            // be executed. The folder opens in Finder.
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
    spawn_open(cmd, &what).await
}

/// First code editor from `validate::EDITORS` with its CLI installed, and where it is. Full IDEs
/// (`idea`, `webstorm`, `fleet`) are left out: too heavy to open just to look at a file.
fn detect_editor() -> Option<(&'static str, std::path::PathBuf)> {
    validate::EDITORS
        .iter()
        .filter(|e| !matches!(**e, "idea" | "webstorm" | "fleet"))
        .find_map(|e| claude_bin::resolve_bin(e).map(|bin| (*e, bin)))
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
    // More concurrency may free up room for queued runs.
    kick(&state.0);
    Ok(s)
}
