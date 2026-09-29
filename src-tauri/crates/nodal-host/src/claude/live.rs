//! Single-flight, TTL-bound cache of the raw `claude agents --json --all` stdout: one
//! `AgentsRaw` per `HostClaudeCli` instance, shared by `cli::list_runs` and
//! `activity::list_agents` (they parse the same stdout into different types) so a call to
//! either one doesn't spawn its own `claude agents` process. `program`: test-only override
//! for a fake `claude`, like `chats.rs`'s.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::process::Command;
use tokio::sync::Mutex;

use super::bin as claude_bin;

const TTL: Duration = Duration::from_secs(2);
const LIST_TIMEOUT: Duration = Duration::from_secs(15);

struct Snapshot {
    at: Instant,
    stdout: Arc<str>,
}

pub struct AgentsRaw {
    /// `None`: the real `claude`. Tests point it at a fake.
    program: Option<PathBuf>,
    cache: Mutex<Option<Snapshot>>,
}

impl AgentsRaw {
    pub fn new() -> Self {
        Self { program: None, cache: Mutex::new(None) }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_program(program: PathBuf) -> Self {
        Self { program: Some(program), cache: Mutex::new(None) }
    }

    fn command(&self) -> Result<Command, String> {
        match &self.program {
            Some(p) => Ok(Command::new(p)),
            None => claude_bin::claude_command(),
        }
    }

    /// Raw stdout of `claude agents --json --all`: single-flight (concurrent callers wait on
    /// the same in-flight fetch) and cached for `TTL`.
    pub async fn get(&self) -> Result<Arc<str>, String> {
        let mut guard = self.cache.lock().await;
        if let Some(s) = guard.as_ref() {
            if s.at.elapsed() < TTL {
                return Ok(s.stdout.clone());
            }
        }
        let mut cmd = self.command()?;
        cmd.args(["agents", "--json", "--all"]);
        let out = claude_bin::output_with_timeout(cmd, LIST_TIMEOUT, "`claude agents`").await?;
        if !out.status.success() {
            return Err(format!("`claude agents` failed: {}", claude_bin::error_text(&out)));
        }
        let stdout: Arc<str> = Arc::from(String::from_utf8_lossy(&out.stdout).into_owned());
        *guard = Some(Snapshot { at: Instant::now(), stdout: stdout.clone() });
        Ok(stdout)
    }

    /// Forces the next `get` to spawn again: called after `launch_bg`/`stop` go through, so a
    /// state change is visible on the next read instead of waiting out `TTL`.
    pub async fn invalidate(&self) {
        *self.cache.lock().await = None;
    }
}

impl Default for AgentsRaw {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
