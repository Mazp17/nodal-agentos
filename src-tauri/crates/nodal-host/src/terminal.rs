//! Actions on background sessions that don't go through the queue: attach in Terminal.app
//! (`claude attach`) and open Terminal in a folder. The other way to attach, a `claude
//! attach` in an in-app pty for the drawer's embedded terminal, lives in `pty`.

use std::path::Path;
use std::time::Duration;

use nodal_domain::execution::worktree::is_valid_run_id;

use crate::claude::bin as claude_bin;

const OSASCRIPT_TIMEOUT: Duration = Duration::from_secs(15);

/// Shell single quotes: `'...'` with `'` → `'\''`.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// AppleScript string literal.
fn applescript_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', r"\\").replace('"', "\\\""))
}

/// `-e` lines for osascript: opens a Terminal window with `claude attach <id>`.
/// `run_id` is already validated; the binary path is quoted anyway in case it has spaces.
fn attach_script(claude: &Path, run_id: &str) -> Vec<String> {
    let cmd = format!("{} attach {}", shell_quote(&claude.to_string_lossy()), run_id);
    vec![
        "tell application \"Terminal\"".into(),
        "activate".into(),
        format!("do script {}", applescript_string(&cmd)),
        "end tell".into(),
    ]
}

async fn osascript(lines: Vec<String>) -> Result<(), String> {
    let mut cmd = tokio::process::Command::new("/usr/bin/osascript");
    for line in lines {
        cmd.arg("-e").arg(line);
    }
    cmd.stdin(std::process::Stdio::null());
    let out = claude_bin::output_with_timeout(cmd, OSASCRIPT_TIMEOUT, "osascript").await?;
    if !out.status.success() {
        return Err(format!("Couldn't open Terminal: {}", claude_bin::error_text(&out)));
    }
    Ok(())
}

/// Opens Terminal.app running `claude attach <run_id>` on a background session.
pub async fn attach(run_id: &str) -> Result<(), String> {
    if !is_valid_run_id(run_id) {
        return Err(format!("Invalid run id: \"{run_id}\"."));
    }
    if !cfg!(target_os = "macos") {
        return Err("Attach is only available on macOS (it uses Terminal.app).".into());
    }
    let claude = claude_bin::resolve_claude()?;
    osascript(attach_script(&claude, run_id)).await
}

/// `-e` lines for osascript: opens a Terminal window in `dir` and, if `claude` is given,
/// starts it there (to accept the workspace trust dialog).
fn terminal_at_script(dir: &Path, claude: Option<&Path>) -> Vec<String> {
    let mut cmd = format!("cd {}", shell_quote(&dir.to_string_lossy()));
    if let Some(c) = claude {
        cmd.push_str(" && ");
        cmd.push_str(&shell_quote(&c.to_string_lossy()));
    }
    vec![
        "tell application \"Terminal\"".into(),
        "activate".into(),
        format!("do script {}", applescript_string(&cmd)),
        "end tell".into(),
    ]
}

/// Opens Terminal.app in `path` (an existing directory). With `run_claude`, starts
/// `claude` there: useful for accepting the trust dialog of a new repo (or of
/// `~/.nodal/worktrees`, where the tasks' worktrees live).
pub async fn open_at(path: &str, run_claude: Option<bool>) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("Opening Terminal is only available on macOS.".into());
    }
    let raw = Path::new(path.trim());
    if !raw.is_absolute() {
        return Err(format!("The folder must be an absolute path: {path}"));
    }
    // A newline (or another control char) would split the AppleScript line.
    if path.chars().any(char::is_control) {
        return Err("The folder path contains control characters.".into());
    }
    let dir = raw
        .canonicalize()
        .ok()
        .filter(|d| d.is_dir())
        .ok_or_else(|| format!("The folder doesn't exist or isn't a directory: {path}"))?;
    let claude = if run_claude.unwrap_or(false) { Some(claude_bin::resolve_claude()?) } else { None };
    // `canonicalize` resolves symlinks: the final path (and claude's) is checked too.
    let has_control = |p: &Path| p.to_string_lossy().chars().any(char::is_control);
    if has_control(&dir) || claude.as_deref().is_some_and(has_control) {
        return Err("The folder path contains control characters.".into());
    }
    osascript(terminal_at_script(&dir, claude.as_deref())).await
}

#[cfg(test)]
mod tests;
