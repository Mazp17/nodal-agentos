//! Read-only access to Claude Code sessions and runs' transcripts.
//!
//! `list_runs`, `get_run_detail`, `get_agent_transcript` and `get_launch_blocker` work
//! without a database (`claude agents` plus the session files under `~/.claude/projects`).
//! `get_run_transcript` and `external_sessions` need one.

use tauri::{AppHandle, Manager, State};

use crate::activity::{existing_roots, external_sessions_of, list_agents, AppRuns, ExternalSessions};
use crate::db::{with_db, Db};
use crate::domain::Executor;
use crate::runs::claude_fs;
use crate::runs::types::{LaunchBlocker, RunDetail, RunSummary, Transcript};
use crate::util::check_id;
use crate::work::WorkState;

pub use nodal_host::claude::fs::readout::projects_dir;

/// Background sessions (`claude agents --json --all`), most recent first.
#[tauri::command]
pub async fn list_runs() -> Result<Vec<RunSummary>, String> {
    nodal_host::claude::cli::list_runs().await
}

/// Detail of the session's most recent workflow. `None` if the session has no folder on
/// disk yet or didn't launch any workflow.
#[tauri::command]
pub async fn get_run_detail(session_id: String, cwd: String) -> Result<Option<RunDetail>, String> {
    if !claude_fs::is_valid_session_id(&session_id) {
        return Err(format!("Invalid session id: {session_id}"));
    }
    tauri::async_runtime::spawn_blocking(move || -> Result<Option<RunDetail>, String> {
        let projects = projects_dir()?;
        Ok(claude_fs::find_session_dir(&projects, &cwd, &session_id).and_then(|dir| claude_fs::read_run_detail(&dir)))
    })
    .await
    .map_err(|e| format!("Internal error reading the session: {e}"))?
}

/// Says whether the session ended without running its workflow because Claude Code asked to
/// approve it ("Review dynamic workflow before running"). `None` if there's no transcript or
/// that rejection doesn't show up. Reads at most the first 4 MB of the main transcript.
#[tauri::command]
pub async fn get_launch_blocker(session_id: String, cwd: String) -> Result<Option<LaunchBlocker>, String> {
    if !claude_fs::is_valid_session_id(&session_id) {
        return Err(format!("Invalid session id: {session_id}"));
    }
    tauri::async_runtime::spawn_blocking(move || -> Result<Option<LaunchBlocker>, String> {
        let projects = projects_dir()?;
        Ok(claude_fs::find_session_jsonl(&projects, &cwd, &session_id)
            .and_then(|p| claude_fs::read_workflow_review_denial(&p))
            .map(|workflow| LaunchBlocker::WorkflowReview { workflow }))
    })
    .await
    .map_err(|e| format!("Internal error reading the session: {e}"))?
}

/// Transcript of a workflow subagent: prompt, conversation (clipped) and output.
/// `limit`: max number of items to return, the most recent ones (default 200, max 2000).
/// `None` if the agent has no file yet.
#[tauri::command]
pub async fn get_agent_transcript(
    session_id: String,
    cwd: String,
    run_id: String,
    agent_id: String,
    limit: Option<u32>,
) -> Result<Option<Transcript>, String> {
    if !claude_fs::is_valid_session_id(&session_id) {
        return Err(format!("Invalid session id: {session_id}"));
    }
    if !run_id.starts_with("wf_") || !claude_fs::is_valid_path_id(&run_id) {
        return Err(format!("Invalid workflow run id: {run_id}"));
    }
    if !claude_fs::is_valid_path_id(&agent_id) {
        return Err(format!("Invalid agent id: {agent_id}"));
    }
    let limit = limit
        .unwrap_or(claude_fs::TRANSCRIPT_DEFAULT_LIMIT)
        .clamp(1, claude_fs::TRANSCRIPT_MAX_LIMIT) as usize;
    tauri::async_runtime::spawn_blocking(move || -> Result<Option<Transcript>, String> {
        let projects = projects_dir()?;
        let Some(dir) = claude_fs::find_session_dir(&projects, &cwd, &session_id) else {
            return Ok(None);
        };
        claude_fs::read_agent_transcript(&dir, &run_id, &agent_id, limit)
    })
    .await
    .map_err(|e| format!("Internal error reading the transcript: {e}"))?
}

async fn db<T, F>(state: &WorkState, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&mut crate::db::Connection) -> Result<T, String> + Send + 'static,
{
    Ok(with_db(&state.0.db, move |c| f(c).map_err(crate::db::DbError::Invalid)).await?)
}

/// Transcript of an agent, Claude or reviewer run (the main session). Workflows have one per
/// agent: `get_agent_transcript`. `None` if the session has no file yet.
/// `limit`: most recent items (default 200, max 2000).
#[tauri::command]
pub async fn get_run_transcript(state: State<'_, WorkState>, run_id: String, limit: Option<u32>) -> Result<Option<Transcript>, String> {
    check_id(&run_id, "run")?;
    let run = db(&state, move |c| Ok(crate::db::queries::runs::get(c, &run_id)?)).await?;
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
    crate::util::blocking(move || {
        let Some(path) = claude_fs::find_session_jsonl(&claude_dir.join("projects"), &run.cwd, &sid) else {
            return Ok(None);
        };
        claude_fs::read_session_transcript(&path, &run.id, Some(label), run.options.model.clone(), limit)
    })
    .await
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Runs launched by the app (`runs` table). If the database isn't available, nothing is marked.
async fn app_run_refs(app: &AppHandle) -> AppRuns {
    let mut refs = AppRuns::default();
    let Some(db) = app.try_state::<Db>() else { return refs };
    match with_db(&db, |c| crate::db::queries::runs::launched_refs(c)).await {
        Ok(list) => {
            for (r, s) in list {
                refs.add(r, s);
            }
        }
        Err(e) => eprintln!("activity: {e}"),
    }
    refs
}

/// Claude Code sessions started outside the app in the project's repos (`None`: every
/// project's), with a single `claude agents`.
#[tauri::command]
pub async fn external_sessions(app: AppHandle, project_id: Option<String>) -> Result<ExternalSessions, String> {
    if let Some(id) = &project_id {
        check_id(id, "project")?;
    }
    let db = app.try_state::<Db>().ok_or("The database isn't available.")?.inner().clone();
    let repos: Vec<(String, std::path::PathBuf)> = with_db(&db, move |c| {
        if let Some(pid) = &project_id {
            crate::db::queries::projects::get(c, pid)?;
        }
        // Every project: skip archived ones (their runs don't show on Runs either).
        let live: Option<std::collections::HashSet<String>> = match project_id {
            Some(_) => None,
            None => Some(crate::db::queries::projects::list(c, false)?.into_iter().map(|p| p.id).collect()),
        };
        Ok(crate::db::queries::repos::list(c, project_id.as_deref())?
            .into_iter()
            .filter(|r| live.as_ref().is_none_or(|l| l.contains(&r.project_id)))
            .map(|r| (r.id, std::path::PathBuf::from(r.path)))
            .collect())
    })
    .await?;
    if repos.is_empty() {
        return Ok(ExternalSessions { repos: vec![], generated_at: now_ms() });
    }
    let agents = list_agents().await?;
    let refs = app_run_refs(&app).await;
    tauri::async_runtime::spawn_blocking(move || {
        let projects = crate::runs::claude_fs::claude_config_dir()
            .ok_or("Couldn't locate the Claude Code folder ($HOME is not set).")?
            .join("projects");
        Ok(external_sessions_of(&repos, &agents, &projects, &refs, now_ms(), existing_roots))
    })
    .await
    .map_err(|e| format!("Internal error reading activity: {e}"))?
}

#[cfg(test)]
mod tests;
