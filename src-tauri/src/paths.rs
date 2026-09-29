//! App data folder (database, plans). Debug builds use `<identifier>.dev` next to it.

use std::path::PathBuf;

use nodal_mcp_proto::paths::dev_sibling;

/// Debug builds (`pnpm tauri dev`) keep their own database, worktrees and keychain entries,
/// so they never touch the data of an installed Nodal.
pub const DEV: bool = cfg!(debug_assertions);

pub fn data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    use tauri::Manager;
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Couldn't find the app data folder: {e}"))?;
    Ok(if DEV { dev_sibling(&dir) } else { dir })
}

#[cfg(test)]
mod tests;
