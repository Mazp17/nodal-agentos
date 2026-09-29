//! Shared utilities: time, ids, paths, git and blocking I/O.
//!
//! `new_id`, `is_valid_id`, `check_id` and `clip_chars` moved to `nodal_domain::util`;
//! re-exported so current uses don't break.

pub mod git;
pub mod paths;

pub use nodal_domain::util::*;

/// Moved to `nodal_host::adapters::SystemClock`; re-exported so current uses don't break.
pub fn now_ms() -> i64 {
    use nodal_domain::ports::Clock;
    nodal_host::adapters::SystemClock.now_ms()
}

/// Runs `f` on a blocking thread (disk, git) without stalling the runtime.
pub async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| format!("Internal error: {e}"))?
}

/// Moved to `nodal_host::plans::write_atomic`; re-exported so current uses don't break.
/// Wave 3c's `Execution::cancel_run` calls `env.plans.write_atomic` (the port) directly;
/// nothing in this crate reaches the bridge anymore.
#[allow(unused_imports)]
pub use nodal_host::plans::write_atomic;
