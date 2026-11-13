//! The `claude` CLI: its version.

use std::time::Duration;

use super::bin as claude_bin;

/// Version of the `claude` CLI: minimal proof that the core can invoke it. Uses the same
/// resolver as the rest of the host, so it also works when opened from Finder. Shell-only
/// free function: the shell wraps this as its `claude_version` command.
pub async fn version() -> Result<String, String> {
    let mut cmd = claude_bin::claude_command()?;
    cmd.arg("--version");
    let out =
        claude_bin::output_with_timeout(cmd, Duration::from_secs(10), "claude --version").await?;
    if !out.status.success() {
        return Err(claude_bin::error_text(&out));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
