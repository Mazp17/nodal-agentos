//! Opens a terminal in a folder: Terminal.app (`osascript`) on macOS, Windows Terminal or `cmd` on Windows.

use std::path::Path;
use crate::claude::bin as claude_bin;

#[cfg(target_os = "macos")]
const OSASCRIPT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Shell single quotes: `'...'` with `'` → `'\''`.
#[cfg(target_os = "macos")]
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// AppleScript string literal.
#[cfg(target_os = "macos")]
fn applescript_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', r"\\").replace('"', "\\\""))
}

#[cfg(target_os = "macos")]
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

/// `-e` lines for osascript: opens a Terminal window in `dir` and, if `claude` is given,
/// starts it there (to accept the workspace trust dialog).
#[cfg(target_os = "macos")]
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

/// Program and argv to open a terminal window in `dir` on Windows: Windows Terminal when
/// `wt` is given, else a new `cmd` console via `start` (its first quoted arg is the title,
/// hence the empty `""`). With `claude`, runs it there.
#[cfg(any(windows, test))]
fn windows_terminal_argv(
    wt: Option<&Path>,
    dir: &Path,
    claude: Option<&Path>,
) -> (std::ffi::OsString, Vec<std::ffi::OsString>) {
    let mut args: Vec<std::ffi::OsString> = Vec::new();
    let program = if let Some(wt) = wt {
        args.extend(["-d".into(), dir.into()]);
        args.extend(claude.map(Into::into));
        wt.into()
    } else {
        args.extend(["/C", "start", "", "/D"].map(Into::into));
        args.extend([dir.into(), "cmd.exe".into()]);
        args.extend(claude.into_iter().flat_map(|c| ["/K".into(), c.into()]));
        "cmd.exe".into()
    };
    (program, args)
}

/// Opens a terminal in `path` (an existing directory). With `run_claude`, starts
/// `claude` there: useful for accepting the trust dialog of a new repo (or of
/// `~/.nodal/worktrees`, where the tasks' worktrees live).
pub async fn open_at(path: &str, run_claude: Option<bool>) -> Result<(), String> {
    if !cfg!(any(target_os = "macos", windows)) {
        return Err("Opening a terminal is only supported on macOS and Windows.".into());
    }
    let raw = Path::new(path.trim());
    if !raw.is_absolute() {
        return Err(format!("The folder must be an absolute path: {path}"));
    }
    // A newline (or another control char) would split the AppleScript line.
    if path.chars().any(char::is_control) {
        return Err("The folder path contains control characters.".into());
    }
    let dir = dunce::canonicalize(raw)
        .ok()
        .filter(|d| d.is_dir())
        .ok_or_else(|| format!("The folder doesn't exist or isn't a directory: {path}"))?;
    let claude = if run_claude.unwrap_or(false) { Some(claude_bin::resolve_claude()?) } else { None };
    // `canonicalize` resolves symlinks: the final path (and claude's) is checked too.
    let has_control = |p: &Path| p.to_string_lossy().chars().any(char::is_control);
    if has_control(&dir) || claude.as_deref().is_some_and(has_control) {
        return Err("The folder path contains control characters.".into());
    }
    #[cfg(target_os = "macos")]
    return osascript(terminal_at_script(&dir, claude.as_deref())).await;
    #[cfg(windows)]
    {
        let wt = claude_bin::resolve_bin("wt");
        let (program, args) = windows_terminal_argv(wt.as_deref(), &dir, claude.as_deref());
        // Detached: the terminal outlives this call, so it isn't waited for.
        std::process::Command::new(program)
            .args(args)
            .spawn()
            .map(drop)
            .map_err(|e| format!("Couldn't open a terminal: {e}"))
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (dir, claude);
        Err("unreachable".into())
    }
}

#[cfg(test)]
mod tests;
