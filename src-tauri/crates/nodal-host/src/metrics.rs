//! Process-wide atomic counters for child-process spawns, for the shell's structured logs
//! and the measurement script (`scripts/perf-measure.sh`). Not persisted, reset on restart.
//!
//! Counted at the single chokepoints for each process kind: [`claude_command`] (built right
//! before every `claude` spawn in `cli.rs`/`activity.rs`/`chats.rs`'s production path — see
//! `claude::bin`) and [`git::run`](crate::git::run) (every `git` spawn). Both are rewritten by
//! P06/P14: keep these calls when that lands.

use std::sync::atomic::{AtomicU64, Ordering};

static CLAUDE_SPAWNS: AtomicU64 = AtomicU64::new(0);
static GIT_SPAWNS: AtomicU64 = AtomicU64::new(0);

/// Call once per `claude` spawn (in `claude::bin::claude_command`).
pub fn record_claude_spawn() -> u64 {
    let total = CLAUDE_SPAWNS.fetch_add(1, Ordering::Relaxed) + 1;
    tracing::debug!(target: "nodal_host::spawn", kind = "claude", total, "claude spawn");
    total
}

/// Call once per `git` spawn (in `git::run`).
pub fn record_git_spawn() -> u64 {
    let total = GIT_SPAWNS.fetch_add(1, Ordering::Relaxed) + 1;
    tracing::debug!(target: "nodal_host::spawn", kind = "git", total, "git spawn");
    total
}

pub fn claude_spawn_count() -> u64 {
    CLAUDE_SPAWNS.load(Ordering::Relaxed)
}

pub fn git_spawn_count() -> u64 {
    GIT_SPAWNS.load(Ordering::Relaxed)
}

/// Test-only: counters are process-global statics, so tests that assert on them must reset
/// first to avoid bleeding into each other under `cargo test`'s shared-process runner.
#[cfg(any(test, feature = "test-support"))]
pub fn reset_for_test() {
    CLAUDE_SPAWNS.store(0, Ordering::Relaxed);
    GIT_SPAWNS.store(0, Ordering::Relaxed);
}

#[cfg(test)]
mod tests;
