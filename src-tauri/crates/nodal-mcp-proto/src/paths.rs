//! Paths `nodal-mcp` needs without a running Tauri app: the home and the app's data folder.
//! No tauri dependency, so this crate compiles without pulling it in.

use std::path::{Path, PathBuf};

pub fn home() -> Option<PathBuf> {
    // Honors $HOME on Unix and resolves the profile folder on Windows.
    std::env::home_dir()
}

/// Debug builds (`pnpm tauri dev`) keep their own database, worktrees and keychain entries,
/// so they never touch the data of an installed Nodal. Kept separate from the shell crate's
/// `util::paths::DEV` so this crate doesn't depend on it.
pub const DEV: bool = cfg!(debug_assertions);

/// `identifier` of `tauri.conf.json`: names the app data folder.
pub const APP_IDENTIFIER: &str = "io.github.mazp17.nodal";

/// `data_dir` without a running Tauri app, for `nodal-mcp`. Same folder Tauri's
/// `app_data_dir()` resolves on each OS.
pub fn data_dir_standalone() -> Result<PathBuf, String> {
    let dir = data_base()?.join(APP_IDENTIFIER);
    Ok(if DEV { dev_sibling(&dir) } else { dir })
}

#[cfg(target_os = "macos")]
fn data_base() -> Result<PathBuf, String> {
    let home = home().ok_or_else(|| "The home folder is not set.".to_string())?;
    Ok(home.join("Library/Application Support"))
}

#[cfg(windows)]
fn data_base() -> Result<PathBuf, String> {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| "%APPDATA% is not set.".to_string())
}

#[cfg(not(any(target_os = "macos", windows)))]
fn data_base() -> Result<PathBuf, String> {
    if let Some(x) = std::env::var_os("XDG_DATA_HOME").filter(|x| !x.is_empty()) {
        return Ok(PathBuf::from(x));
    }
    let home = home().ok_or_else(|| "The home folder is not set.".to_string())?;
    Ok(home.join(".local/share"))
}

/// Where the MCP server listens: a Unix socket inside the app data folder.
#[cfg(unix)]
pub fn mcp_socket(data_dir: &Path) -> PathBuf {
    data_dir.join("mcp.sock")
}

/// Where the MCP server listens: a named pipe derived from the data folder.
#[cfg(windows)]
pub fn mcp_socket(data_dir: &Path) -> PathBuf {
    PathBuf::from(pipe_name(data_dir))
}

/// `\\.\pipe\nodal-mcp-<data dir>`: the data folder tells dev and prod builds apart. A pipe
/// name holds at most 256 characters, so a long path keeps its tail (where dev/prod differ).
pub fn pipe_name(data_dir: &Path) -> String {
    const PREFIX: &str = r"\\.\pipe\nodal-mcp-";
    let id: Vec<char> = data_dir
        .to_string_lossy()
        .chars()
        .map(|c| {
            if matches!(c, '\\' | '/' | ':') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let tail: String = id[id.len().saturating_sub(200)..].iter().collect();
    format!("{PREFIX}{tail}")
}

/// `/x/io.github.mazp17.nodal` → `/x/io.github.mazp17.nodal.dev`.
pub fn dev_sibling(dir: &Path) -> PathBuf {
    let mut name = dir.file_name().unwrap_or_default().to_os_string();
    name.push(".dev");
    dir.with_file_name(name)
}

#[cfg(test)]
mod tests;
