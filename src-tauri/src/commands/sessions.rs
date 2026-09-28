//! Sessions Tauri commands (signatures in `src/domain/api.ts`): read-only access to Claude
//! Code sessions and runs' transcripts.
//!
//! `get_run_detail`, `get_launch_blocker` and `get_agent_transcript` work without a database,
//! through the always-registered `SessionReader`. `get_run_transcript` and `external_sessions`
//! need one, through `Arc<App>`.

use std::sync::Arc;

use tauri::{AppHandle, Manager, State};

use nodal_app::sessions::SessionReader;
use nodal_app::App;
use nodal_domain::model::activity::ExternalSessions;
use nodal_domain::model::claude::{LaunchBlocker, RunDetail, RunSummary, Transcript};

use super::CommandError;

/// Background sessions (`claude agents --json --all`), most recent first.
///
/// Kept without a `State` (unlike the other three DB-free session commands, below):
/// `commands::execution::work_summary` and `work::pump` still call this as a plain function,
/// with no `State` argument (see the report for wave 3a). Once wave 3c moves those call sites
/// onto `core.claude`/`SessionReader`, this can switch to `State<'_, Arc<SessionReader>>` like
/// the rest and return `CommandError`.
#[tauri::command]
pub async fn list_runs() -> Result<Vec<RunSummary>, String> {
    nodal_host::claude::cli::list_runs().await
}

/// Detail of the session's most recent workflow. `None` if the session has no folder on
/// disk yet or didn't launch any workflow.
#[tauri::command]
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
pub async fn get_run_transcript(
    state: State<'_, Arc<App>>,
    run_id: String,
    limit: Option<u32>,
) -> Result<Option<Transcript>, CommandError> {
    Ok(state.sessions.get_run_transcript(run_id, limit).await?)
}

/// Claude Code sessions started outside the app in the project's repos (`None`: every
/// project's), with a single `claude agents`.
#[tauri::command]
pub async fn external_sessions(app: AppHandle, project_id: Option<String>) -> Result<ExternalSessions, CommandError> {
    let app = app.try_state::<Arc<App>>().ok_or_else(|| "The database isn't available.".to_string())?;
    Ok(app.sessions.external_sessions(project_id).await?)
}
