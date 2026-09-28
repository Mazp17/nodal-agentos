//! The `claude` CLI: launching, listing and stopping background runs.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::runtime::Handle;
use tokio::sync::mpsc;

use nodal_domain::execution::worktree::is_valid_run_id;
use nodal_domain::model::claude::{ExtraFlags, LaunchError, RunRef, RunSummary};
use nodal_domain::model::LaunchOptions;

use super::bin as claude_bin;
use super::fs::agents_json::parse_agents_json;
use super::fs::bg_output::{parse_bare_id, parse_bg_line};

const LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);
const LIST_TIMEOUT: Duration = Duration::from_secs(15);
const STOP_TIMEOUT: Duration = Duration::from_secs(20);

/// Forwards each line of a child stream to the channel.
fn forward_lines<R: AsyncRead + Unpin + Send + 'static>(stream: R, tx: mpsc::UnboundedSender<String>, rt: &Handle) {
    rt.spawn(async move {
        let mut lines = BufReader::new(stream).lines();
        // Drain until EOF even if nobody listens anymore: closing the pipe would give EPIPE to
        // `claude` (or to the background session, if it inherited the pipe).
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = tx.send(line);
        }
    });
}

/// Launches `claude --bg [flags] -- <prompt>` in `cwd` and returns the session's short id.
/// Everything goes as separate arguments (no shell), so there's nothing to escape; the
/// flags are validated against the allowed lists in `options`. The `--` ends the variadic
/// options before the prompt.
pub async fn launch_bg(
    cwd: String,
    prompt: String,
    opts: &LaunchOptions,
    extra: &ExtraFlags,
    rt: &Handle,
) -> Result<RunRef, LaunchError> {
    let dir = Path::new(&cwd);
    if !dir.is_absolute() {
        return Err(format!("The folder must be an absolute path: {cwd}").into());
    }
    if !dir.is_dir() {
        return Err(format!("The folder doesn't exist or isn't a directory: {cwd}").into());
    }
    if prompt.trim().is_empty() {
        return Err("The prompt is empty.".into());
    }
    // `claude` would read it as an option, not as the prompt.
    if prompt.trim_start().starts_with('-') {
        return Err("The prompt can't start with \"-\".".into());
    }
    let flags = nodal_domain::execution::options::to_args(opts)?;

    let mut cmd = claude_bin::claude_command()?;
    cmd.arg("--bg")
        .args(&flags)
        .args(extra.to_args())
        .arg("--")
        .arg(&prompt)
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("Couldn't run `claude --bg`: {e}"))?;

    // Don't wait for EOF: seeing the `backgrounded · <id>` line on either stream is enough
    // (there's no guarantee the background session releases the pipes).
    let (tx, mut rx) = mpsc::unbounded_channel();
    if let Some(out) = child.stdout.take() {
        forward_lines(out, tx.clone(), rt);
    }
    if let Some(err) = child.stderr.take() {
        forward_lines(err, tx, rt);
    }
    let found = tokio::time::timeout(LAUNCH_TIMEOUT, async {
        let mut seen = String::new();
        while let Some(line) = rx.recv().await {
            if let Some(id) = parse_bg_line(&line) {
                return Ok(id);
            }
            seen.push_str(&line);
            seen.push('\n');
        }
        Err(seen)
    })
    .await;

    match found {
        Ok(Ok(id)) => {
            // Reap the process when it exits, without blocking the response.
            rt.spawn(async move {
                let _ = child.wait().await;
            });
            Ok(RunRef { id, cwd })
        }
        Ok(Err(seen)) => {
            let status = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
            if let Some(id) = parse_bare_id(&seen) {
                return Ok(RunRef { id, cwd });
            }
            let _ = child.start_kill();
            let code = match status {
                Ok(Ok(s)) => s.code().map(|c| format!(" (exit code {c})")).unwrap_or_default(),
                _ => String::new(),
            };
            let seen: String = seen.trim().chars().take(500).collect();
            Err(format!("`claude --bg` exited without returning the session id{code}: {seen}").into())
        }
        Err(_) => {
            let _ = child.start_kill();
            Err(LaunchError {
                message: format!(
                    "`claude --bg` didn't return the session id within {} s. It may have launched anyway: check the runs list before retrying.",
                    LAUNCH_TIMEOUT.as_secs()
                ),
                timed_out: true,
            })
        }
    }
}

/// Version of the `claude` CLI: minimal proof that the core can invoke it. Uses the same
/// resolver as the rest of the host, so it also works when opened from Finder. Shell-only
/// free function: the shell wraps this as its `claude_version` command.
pub async fn version() -> Result<String, String> {
    let mut cmd = claude_bin::claude_command()?;
    cmd.arg("--version");
    let out = claude_bin::output_with_timeout(cmd, Duration::from_secs(10), "claude --version").await?;
    if !out.status.success() {
        return Err(claude_bin::error_text(&out));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Background sessions (`claude agents --json --all`), most recent first.
pub async fn list_runs() -> Result<Vec<RunSummary>, String> {
    let mut cmd = claude_bin::claude_command()?;
    cmd.args(["agents", "--json", "--all"]);
    let out = claude_bin::output_with_timeout(cmd, LIST_TIMEOUT, "`claude agents`").await?;
    if !out.status.success() {
        return Err(format!("`claude agents` failed: {}", claude_bin::error_text(&out)));
    }
    parse_agents_json(&String::from_utf8_lossy(&out.stdout))
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
