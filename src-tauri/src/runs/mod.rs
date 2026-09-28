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

use types::{LaunchOptions, RunRef};

/// Moved to `nodal_domain::model::claude`; re-exported so current uses don't break.
pub use nodal_domain::model::claude::{ExtraFlags, LaunchError, SessionReadout};

pub use nodal_host::claude::fs::readout::{read_session, session_tokens};
/// Only the bridge is left; nothing in this crate calls it directly anymore.
#[allow(unused_imports)]
pub use nodal_host::claude::fs::readout::projects_dir;

/// `list_runs` moved to `SRC/commands/sessions.rs`, which keeps this exact (state-free)
/// signature: `commands::execution::work_summary` and `work::pump` call it directly, without
/// going through Tauri's state injection (see wave 3a's report for why).
pub use crate::commands::sessions::list_runs;

/// `get_run_detail`'s validation and lookup moved to `nodal_app::sessions::SessionReader`,
/// behind the Tauri command's new `State<'_, Arc<SessionReader>>`. Kept here, state-free and
/// with the same body as before, only for this module's `real_list_and_detail` smoke test
/// (like `launch_with`, below, it has no other caller, hence `#[cfg(test)]`).
#[cfg(test)]
pub async fn get_run_detail(session_id: String, cwd: String) -> Result<Option<types::RunDetail>, String> {
    if !claude_fs::is_valid_session_id(&session_id) {
        return Err(format!("Invalid session id: {session_id}"));
    }
    tauri::async_runtime::spawn_blocking(move || -> Result<Option<types::RunDetail>, String> {
        let projects = claude_fs::projects_dir()?;
        Ok(claude_fs::find_session_dir(&projects, &cwd, &session_id).and_then(|dir| claude_fs::read_run_detail(&dir)))
    })
    .await
    .map_err(|e| format!("Internal error reading the session: {e}"))?
}

/// Only the bridge is left; nothing in this crate calls them directly anymore.
#[allow(unused_imports)]
pub use crate::commands::sessions::{get_agent_transcript, get_launch_blocker};

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

#[cfg(test)]
mod tests;
