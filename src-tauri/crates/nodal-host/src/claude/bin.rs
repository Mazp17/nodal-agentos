//! Resolution of the `claude` binary and execution with a timeout.
//!
//! An app opened from Finder doesn't inherit the shell's PATH: so it looks in PATH and,
//! failing that, in the known install locations; and the child gets an extended PATH so
//! that `claude` can in turn find git, node, etc.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

use tokio::process::Command;

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Directories where `claude` (and the tools it uses) usually live outside Finder's PATH.
fn extra_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(h) = home() {
        dirs.push(h.join(".local").join("bin"));
        dirs.push(h.join(".claude").join("local"));
    }
    for d in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"] {
        dirs.push(PathBuf::from(d));
    }
    dirs
}

fn is_executable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

/// Path of an executable: PATH first, then the known directories.
pub fn resolve_bin(name: &str) -> Option<PathBuf> {
    let from_path = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    from_path
        .into_iter()
        .chain(extra_dirs())
        .map(|d| d.join(name))
        .find(|c| is_executable(c))
}

/// Path of the `claude` binary: PATH first, then `~/.local/bin/claude` and the like.
/// Resolved once per process (P06): the PATH walk below does a `metadata` call per
/// candidate, and PATH doesn't change while the app is running.
pub fn resolve_claude() -> Result<PathBuf, String> {
    static CLAUDE_BIN: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    CLAUDE_BIN
        .get_or_init(|| {
            resolve_bin("claude").ok_or_else(|| {
                "Couldn't find the `claude` executable in PATH or ~/.local/bin. Is Claude Code installed?"
                    .to_string()
            })
        })
        .clone()
}

/// Path of the `git` binary, resolved once per process the same way as [`resolve_claude`]
/// (used by `git::run`, P06's other chokepoint).
pub fn resolve_git() -> Result<PathBuf, String> {
    static GIT_BIN: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    GIT_BIN
        .get_or_init(|| resolve_bin("git").ok_or_else(|| "Couldn't find `git` in PATH.".to_string()))
        .clone()
}

/// `PATH` extended with [`extra_dirs`], built once per process: every `claude` and `git`
/// spawn sets this as the child's `PATH`, and it doesn't change while the app is running.
pub fn augmented_path() -> OsString {
    static AUGMENTED: OnceLock<OsString> = OnceLock::new();
    AUGMENTED
        .get_or_init(|| {
            let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
                .map(|p| std::env::split_paths(&p).collect())
                .unwrap_or_default();
            for d in extra_dirs() {
                if !dirs.contains(&d) {
                    dirs.push(d);
                }
            }
            std::env::join_paths(dirs).unwrap_or_default()
        })
        .clone()
}

/// `claude` ready to configure: no stdin and an extended PATH. The chokepoint every `claude`
/// spawn goes through in production (cli.rs, activity.rs, chats.rs).
pub fn claude_command() -> Result<Command, String> {
    let mut cmd = Command::new(resolve_claude()?);
    cmd.env("PATH", augmented_path()).stdin(Stdio::null());
    crate::metrics::record_claude_spawn();
    Ok(cmd)
}

/// Runs and waits for the full output; if `limit` passes, kills the process.
#[tracing::instrument(skip(cmd, limit), level = "info")]
pub async fn output_with_timeout(
    mut cmd: Command,
    limit: Duration,
    what: &str,
) -> Result<Output, String> {
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = cmd
        .spawn()
        .map_err(|e| format!("Couldn't run {what}: {e}"))?;
    match tokio::time::timeout(limit, child.wait_with_output()).await {
        Ok(Ok(out)) => Ok(out),
        Ok(Err(e)) => Err(format!("{what} failed: {e}")),
        // Dropping the future drops the child, and `kill_on_drop` kills it.
        Err(_) => Err(format!(
            "{what} didn't respond within {} s and was cancelled.",
            limit.as_secs()
        )),
    }
}

/// stderr (or stdout if stderr is empty), trimmed, for error messages.
pub fn error_text(out: &Output) -> String {
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if err.is_empty() {
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    } else {
        err
    }
}

#[cfg(test)]
mod tests;
