//! Host commands: shell-only I/O that talks to `claude`/`git` on disk or spawns a terminal,
//! with no database involved.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::runs::{claude_bin, claude_trust::repo_trust_blocking};
use crate::util::blocking;
use crate::util::paths::{git_repos_under, git_root_of};

/// Version of the `claude` CLI: minimal proof that the core can invoke it.
/// Uses the same resolver as runs, so it also works when opened from Finder.
#[tauri::command]
pub async fn claude_version() -> Result<String, String> {
    let mut cmd = claude_bin::claude_command()?;
    cmd.arg("--version");
    let out = claude_bin::output_with_timeout(cmd, Duration::from_secs(10), "claude --version").await?;
    if !out.status.success() {
        return Err(claude_bin::error_text(&out));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `git` version (`git version 2.x`), using the same resolver as the rest of the app.
#[tauri::command]
pub async fn git_version() -> Result<String, String> {
    blocking(|| {
        let out = nodal_host::git::ok(Path::new("/"), &["--version"])?;
        Ok(out.trim().to_string())
    })
    .await
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

/// Whether Claude Code already trusts the folder (no trust dialog on launch).
#[tauri::command]
pub async fn repo_trust(path: String) -> Result<nodal_host::claude::trust::RepoTrust, String> {
    let path = PathBuf::from(path.trim());
    blocking(move || repo_trust_blocking(&path, nodal_host::claude::trust::global_config_file().as_deref())).await
}

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

#[cfg(test)]
mod tests;
