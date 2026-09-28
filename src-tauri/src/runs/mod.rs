//! Claude Code background runs: launch them (`claude --bg`), list them
//! (`claude agents`) and follow the workflow they run by reading their session files.
//!
//! `launch_bg`, `list_runs`'s body, `read_session`, `session_tokens` and `projects_dir`
//! moved to `nodal_host::claude::{cli, fs}`; re-exported (or called) here so current uses
//! don't break.

pub mod claude_bin;
pub(crate) mod claude_fs;
pub mod claude_settings;
pub mod claude_trust;
pub mod options;
pub mod pty;
pub mod stream_json;
pub mod terminal;
pub mod types;
pub mod workflows;

use types::{LaunchBlocker, LaunchOptions, RunDetail, RunRef, RunSummary, Transcript};

/// Moved to `nodal_domain::model::claude`; re-exported so current uses don't break.
pub use nodal_domain::model::claude::{ExtraFlags, LaunchError, SessionReadout};

pub use nodal_host::claude::fs::readout::{projects_dir, read_session, session_tokens};

fn runtime_handle() -> tokio::runtime::Handle {
    tauri::async_runtime::handle().inner().clone()
}

/// Like `launch_bg`, with the error as text.
#[cfg(test)]
async fn launch_with(cwd: String, prompt: String, opts: &LaunchOptions, extra: &ExtraFlags) -> Result<RunRef, String> {
    launch_bg(cwd, prompt, opts, extra).await.map_err(|e| e.message)
}

/// Launches `claude --bg [flags] -- <prompt>` in `cwd` and returns the session's short id.
/// Moved to `nodal_host::claude::cli::launch_bg`.
pub async fn launch_bg(cwd: String, prompt: String, opts: &LaunchOptions, extra: &ExtraFlags) -> Result<RunRef, LaunchError> {
    nodal_host::claude::cli::launch_bg(cwd, prompt, opts, extra, &runtime_handle()).await
}

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

#[cfg(test)]
mod tests;
