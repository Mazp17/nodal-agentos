//! Actions on background sessions that don't go through the queue: stop (`claude stop`),
//! open in Terminal (`claude attach`) and open Terminal in a folder.

use std::path::Path;
use std::time::Duration;

use super::claude_bin;

const STOP_TIMEOUT: Duration = Duration::from_secs(20);
const OSASCRIPT_TIMEOUT: Duration = Duration::from_secs(15);

/// Short id of a run (`claude --bg` prints hex). It goes into a command and into AppleScript.
pub fn is_valid_run_id(id: &str) -> bool {
    (4..=64).contains(&id.len()) && !id.starts_with('-') && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// `claude stop <id>`.
pub async fn stop(run_id: &str) -> Result<(), String> {
    if !is_valid_run_id(run_id) {
        return Err(format!("Invalid run id: \"{run_id}\"."));
    }
    let mut cmd = claude_bin::claude_command()?;
    cmd.args(["stop", run_id]);
    let out = claude_bin::output_with_timeout(cmd, STOP_TIMEOUT, "`claude stop`").await?;
    if !out.status.success() {
        return Err(format!("`claude stop {run_id}` failed: {}", claude_bin::error_text(&out)));
    }
    Ok(())
}

/// Stops a session by its short id. For queue runs, use `cancel_run` (which also saves the
/// partial patch and applies the transition).
#[tauri::command]
pub async fn stop_run(run_id: String) -> Result<(), String> {
    stop(&run_id).await
}

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

#[tauri::command]
pub async fn attach_run(run_id: String) -> Result<(), String> {
    if !is_valid_run_id(&run_id) {
        return Err(format!("Invalid run id: \"{run_id}\"."));
    }
    if !cfg!(target_os = "macos") {
        return Err("Attach is only available on macOS (it uses Terminal.app).".into());
    }
    let claude = claude_bin::resolve_claude()?;
    osascript(attach_script(&claude, &run_id)).await
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
#[tauri::command]
pub async fn open_terminal_at(path: String, run_claude: Option<bool>) -> Result<(), String> {
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
mod tests {
    use super::*;

    #[test]
    fn run_id_validation() {
        assert!(is_valid_run_id("ddb91222"));
        assert!(is_valid_run_id("abc-123"));
        assert!(!is_valid_run_id("-rf"));
        assert!(!is_valid_run_id("ab"));
        assert!(!is_valid_run_id("abcd\"; rm -rf ~"));
        assert!(!is_valid_run_id("abcd efgh"));
        assert!(!is_valid_run_id("abcd'"));
    }

    #[test]
    fn attach_script_quotes_path() {
        let lines = attach_script(Path::new("/Users/a b/it's \"x\"/claude"), "ddb91222");
        assert_eq!(lines[0], "tell application \"Terminal\"");
        // Shell: '/Users/a b/it'\''s "x"/claude' attach ddb91222, then escaped for AppleScript.
        assert_eq!(lines[2], r#"do script "'/Users/a b/it'\\''s \"x\"/claude' attach ddb91222""#);
    }

    #[test]
    fn terminal_at_script_quotes_path() {
        let lines = terminal_at_script(Path::new("/Users/a b/it's \"x\""), Some(Path::new("/bin/claude")));
        assert_eq!(lines[2], r#"do script "cd '/Users/a b/it'\\''s \"x\"' && '/bin/claude'""#);
        let lines = terminal_at_script(Path::new("/tmp"), None);
        assert_eq!(lines[2], r#"do script "cd '/tmp'""#);
    }
}
