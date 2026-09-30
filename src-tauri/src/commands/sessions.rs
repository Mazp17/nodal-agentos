//! Sessions Tauri commands (signatures in `src/domain/api.ts`): read-only access to Claude
//! Code sessions and runs' transcripts.
//!
//! `get_run_detail`, `get_launch_blocker` and `get_agent_transcript` work without a database,
//! through the always-registered `SessionReader`. `get_run_transcript` and `external_sessions`
//! need one, through `Arc<App>`.

use std::sync::Arc;

use tauri::{AppHandle, Manager, State};

use nodal_app::sessions::SessionReader;
use nodal_app::sources::SourcesHub;
use nodal_app::App;
use nodal_domain::model::activity::ExternalSessions;
use nodal_domain::model::claude::{LaunchBlocker, RunDetail, RunProgress, RunSummary, Transcript};

use super::CommandError;

/// Background sessions (`claude agents --json --all`), most recent first. Goes through
/// `SessionReader` (P01's `LiveSessions` cache underneath), like the other DB-free session
/// commands, instead of spawning its own `claude agents`.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn list_runs(state: State<'_, Arc<SessionReader>>) -> Result<Vec<RunSummary>, CommandError> {
    Ok(state.list_runs().await?)
}

/// Detail of the session's most recent workflow. `None` if the session has no folder on
/// disk yet or didn't launch any workflow.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn get_run_detail(
    state: State<'_, Arc<SessionReader>>,
    session_id: String,
    cwd: String,
) -> Result<Option<RunDetail>, CommandError> {
    Ok(state.get_run_detail(session_id, cwd).await?)
}

/// Says whether the session ended without running its workflow because Claude Code asked to
/// approve it ("Review dynamic workflow before running"). `None` if there's no transcript or
/// that rejection doesn't show up. Reads at most the first 4 MB of the main transcript.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn get_launch_blocker(
    state: State<'_, Arc<SessionReader>>,
    session_id: String,
    cwd: String,
) -> Result<Option<LaunchBlocker>, CommandError> {
    Ok(state.get_launch_blocker(session_id, cwd).await?)
}

/// Transcript of a workflow subagent: prompt, conversation (clipped) and output.
/// `limit`: max number of items to return, the most recent ones (default 200, max 2000).
/// `None` if the agent has no file yet.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn get_agent_transcript(
    state: State<'_, Arc<SessionReader>>,
    session_id: String,
    cwd: String,
    run_id: String,
    agent_id: String,
    limit: Option<u32>,
) -> Result<Option<Transcript>, CommandError> {
    Ok(state.get_agent_transcript(session_id, cwd, run_id, agent_id, limit).await?)
}

/// Transcript of an agent, Claude or reviewer run (the main session). Workflows have one per
/// agent: `get_agent_transcript`. `None` if the session has no file yet. `limit`: most recent
/// items (default 200, max 2000).
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn get_run_transcript(
    state: State<'_, Arc<App>>,
    run_id: String,
    limit: Option<u32>,
) -> Result<Option<Transcript>, CommandError> {
    Ok(state.sessions.get_run_transcript(run_id, limit).await?)
}

/// Tool calls of a live, non-workflow run so far, read with an incremental cursor instead of
/// re-parsing the whole transcript on every poll (P03). `null` if the run has no session yet.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn run_progress(state: State<'_, Arc<App>>, run_id: String) -> Result<Option<RunProgress>, CommandError> {
    Ok(state.sessions.run_progress(run_id).await?)
}

/// Claude Code sessions started outside the app in the project's repos (`None`: every
/// project's), with a single `claude agents`.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn external_sessions(app: AppHandle, project_id: Option<String>) -> Result<ExternalSessions, CommandError> {
    if let Some(id) = &project_id {
        nodal_domain::util::check_id(id, "project")?;
    }
    let Some(state) = app.try_state::<Arc<App>>() else {
        // `App` also needs `$HOME`: with the database open, keep today's message for that case.
        let db_open = app.try_state::<Arc<SourcesHub>>().is_some_and(|hub| hub.db.is_some());
        return Err(if db_open {
            "Couldn't locate the Claude Code folder ($HOME is not set).".to_string()
        } else {
            "The database isn't available.".to_string()
        }
        .into());
    };
    Ok(state.sessions.external_sessions(project_id).await?)
}
