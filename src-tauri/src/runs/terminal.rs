//! Moved to `nodal_host::{claude::cli, terminal}`; re-exported so current uses don't break.

pub use nodal_domain::execution::worktree::is_valid_run_id;
pub use nodal_host::claude::cli::stop;

/// Opens Terminal.app running `claude attach <run_id>` on a background session.
#[tauri::command]
pub async fn attach_run(run_id: String) -> Result<(), String> {
    nodal_host::terminal::attach(&run_id).await
}

/// Opens Terminal.app in `path` (an existing directory). With `run_claude`, starts
/// `claude` there: useful for accepting the trust dialog of a new repo (or of
/// `~/.nodal/worktrees`, where the tasks' worktrees live).
#[tauri::command]
pub async fn open_terminal_at(path: String, run_claude: Option<bool>) -> Result<(), String> {
    nodal_host::terminal::open_at(&path, run_claude).await
}
