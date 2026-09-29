//! Paths `nodal-mcp` needs without a running Tauri app: `$HOME` and the app's data folder.
//! No tauri dependency, so this crate compiles without pulling it in.

use std::path::{Path, PathBuf};

pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Debug builds (`pnpm tauri dev`) keep their own database, worktrees and keychain entries,
/// so they never touch the data of an installed Nodal. Kept separate from the shell crate's
/// `util::paths::DEV` so this crate doesn't depend on it.
pub const DEV: bool = cfg!(debug_assertions);

/// `identifier` of `tauri.conf.json`: names the app data folder.
pub const APP_IDENTIFIER: &str = "io.github.mazp17.nodal";

/// `data_dir` without a running Tauri app, for `nodal-mcp`. Same folder Tauri resolves on macOS.
pub fn data_dir_standalone() -> Result<PathBuf, String> {
    let home = home().ok_or_else(|| "$HOME is not set.".to_string())?;
    let dir = home
        .join("Library/Application Support")
        .join(APP_IDENTIFIER);
    Ok(if DEV { dev_sibling(&dir) } else { dir })
}

/// Unix socket of the MCP server, inside the app data folder.
pub fn mcp_socket(data_dir: &Path) -> PathBuf {
    data_dir.join("mcp.sock")
}

/// `/x/io.github.mazp17.nodal` → `/x/io.github.mazp17.nodal.dev`.
pub fn dev_sibling(dir: &Path) -> PathBuf {
    let mut name = dir.file_name().unwrap_or_default().to_os_string();
    name.push(".dev");
    dir.with_file_name(name)
}

#[cfg(test)]
mod tests;
