//! Paths: `~`, the canonical git root and Nodal's folder in the home directory.

use std::path::PathBuf;

use super::blocking;

/// Moved to `nodal_mcp_proto::paths` (W1), which keeps its own copy of `DEV` (this crate's
/// `DEV` below stays, since it's used well beyond paths: `updates.rs`, `secrets.rs`, `lib.rs`).
pub use nodal_mcp_proto::paths::mcp_socket;
/// Only the bridge is left; nothing in this crate calls it directly anymore (`nodal_home`
/// and the other host-side path functions now use `nodal_host::paths::home` instead).
#[allow(unused_imports)]
pub use nodal_mcp_proto::paths::home;
use nodal_mcp_proto::paths::dev_sibling;
/// Only their old-path bridge is left; nothing in this crate calls them once `mcp::stdio`'s
/// standalone lookup and its test moved to nodal-mcp-proto with it.
#[allow(unused_imports)]
pub use nodal_mcp_proto::paths::{data_dir_standalone, APP_IDENTIFIER};

/// Moved to `nodal_host::paths`; re-exported so current uses don't break.
pub use nodal_host::paths::{canonical_dir, expand_home, git_repos_under, git_root_of, nodal_home, require_git_root};

/// Debug builds (`pnpm tauri dev`) keep their own database, worktrees and keychain entries,
/// so they never touch the data of an installed Nodal.
pub const DEV: bool = cfg!(debug_assertions);

/// App data folder (database, plans). Debug builds use `<identifier>.dev` next to it.
pub fn data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    use tauri::Manager;
    let dir = app.path().app_data_dir().map_err(|e| format!("Couldn't find the app data folder: {e}"))?;
    Ok(if DEV { dev_sibling(&dir) } else { dir })
}

/// Root of the git repo containing `path`, or `None` if it's not inside a repo. Used to
/// offer the root when a subfolder is picked.
#[tauri::command]
pub async fn resolve_git_root(path: String) -> Result<Option<String>, String> {
    blocking(move || Ok(git_root_of(&path)?.map(|p| p.to_string_lossy().into_owned()))).await
}

/// Git repos inside a project's root folder, to offer them for adding.
#[tauri::command]
pub async fn scan_git_repos(root: String) -> Result<Vec<String>, String> {
    blocking(move || Ok(git_repos_under(&root)?.into_iter().map(|p| p.to_string_lossy().into_owned()).collect())).await
}

#[cfg(test)]
pub(crate) mod tests;
