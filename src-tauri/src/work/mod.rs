//! Work: projects, repos and tasks (CRUD), the single run queue, executors,
//! worktrees, automatic review and diff.
//!
//! - `ops`: synchronous CRUD over the database (commands run it with `with_db`);
//! - `launch`: prompt building and run enqueueing;
//! - `queue` and `transitions`: pure queue and state logic;
//! - `pump`: the periodic pass that detects finishes, applies transitions and launches;
//! - `commands`: the Tauri commands.

pub mod commands;
pub mod diff;
pub mod dto;
pub mod executors;
pub mod launch;
pub mod merge;
pub mod ops;
pub mod pump;
pub mod queue;
pub mod report;
pub mod transitions;
pub mod validate;
pub mod worktree;

#[cfg(test)]
pub(crate) mod testutil;

use std::collections::HashSet;
#[cfg(test)]
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Manager};

use crate::db::Db;

const TICK: Duration = Duration::from_secs(5);

/// Moved to `nodal_app::Env` (same field names and methods, plus the ports the old code
/// reached directly); re-exported so current uses don't break.
pub use nodal_app::Env;

/// `Env` for tests: real (blocking, synchronous) host adapters rooted at `data_dir`/
/// `worktrees_root`, so callers keep their own `TempDir` and path names.
#[cfg(test)]
pub(crate) fn test_env(data_dir: PathBuf, worktrees_root: PathBuf, claude_dir: Option<PathBuf>) -> Env {
    Env {
        data_dir,
        worktrees_root,
        claude_dir,
        plans: Arc::new(nodal_host::adapters::HostPlanFiles),
        fs: Arc::new(nodal_host::adapters::HostLocalFs),
        git: Arc::new(nodal_host::adapters::HostGit),
        claude_config: Arc::new(nodal_host::adapters::HostClaudeConfig),
    }
}

pub struct Inner {
    pub db: Db,
    pub env: Env,
    /// Serializes queue passes: only one at a time decides and launches.
    pub pump: tokio::sync::Mutex<()>,
    /// `nodal://changed` to the UI.
    pub events: crate::events::Events,
    /// Error of the last queue pass (`None` if it went fine), for `work_summary`.
    pub pump_error: Mutex<Option<String>>,
    /// Tasks whose worktree is being cleaned up.
    pub cleaning: Cleaning,
}

/// Tasks whose worktree is being deleted. `cleanup_worktree` marks the task and checks that
/// it has no pending runs in the same section while holding the database; `enqueue_work`
/// rejects a marked task in its transaction. That way a launch can't reuse a worktree that is
/// being deleted.
#[derive(Debug, Clone, Default)]
pub struct Cleaning(Arc<Mutex<HashSet<String>>>);

impl Cleaning {
    pub fn contains(&self, task_id: &str) -> bool {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).contains(task_id)
    }

    /// Marks the task; `None` if it's already being cleaned up. The mark goes away with the guard.
    pub fn mark(&self, task_id: &str) -> Option<CleaningGuard> {
        let fresh = self.0.lock().unwrap_or_else(|p| p.into_inner()).insert(task_id.to_string());
        fresh.then(|| CleaningGuard { set: self.clone(), task_id: task_id.to_string() })
    }
}

pub struct CleaningGuard {
    set: Cleaning,
    task_id: String,
}

impl Drop for CleaningGuard {
    fn drop(&mut self) {
        self.set.0.lock().unwrap_or_else(|p| p.into_inner()).remove(&self.task_id);
    }
}

/// `enqueue_work` error for a marked task.
pub const CLEANING_ERR: &str = "The task's worktree is being cleaned up: wait for it to finish.";

#[derive(Clone)]
pub struct WorkState(pub Arc<Inner>);

/// Registers the state and starts the queue.
pub fn init(app: &AppHandle, db: Db) -> Result<(), String> {
    let data_dir = crate::util::paths::data_dir(app)?;
    let env = Env {
        data_dir,
        worktrees_root: crate::util::paths::nodal_home()?.join("worktrees"),
        claude_dir: crate::runs::claude_fs::claude_config_dir(),
        plans: Arc::new(nodal_host::adapters::HostPlanFiles),
        fs: Arc::new(nodal_host::adapters::HostLocalFs),
        git: Arc::new(nodal_host::adapters::HostGit),
        claude_config: Arc::new(nodal_host::adapters::HostClaudeConfig),
    };
    // A `launching` from a previous session may or may not have actually launched.
    if let Ok(mut conn) = db.lock() {
        if let Err(e) = pump::fail_stale_launches(&mut conn, pump::NOTE_APP_CLOSED, crate::util::now_ms()) {
            eprintln!("work: {e}");
        }
    }
    let events = app.try_state::<crate::events::Events>().map(|e| e.inner().clone()).unwrap_or_default();
    let inner = Arc::new(Inner {
        db,
        env,
        pump: tokio::sync::Mutex::new(()),
        events,
        pump_error: Mutex::new(None),
        cleaning: Cleaning::default(),
    });
    app.manage(WorkState(inner.clone()));
    tauri::async_runtime::spawn(async move {
        loop {
            if let Err(e) = pump::pump(&inner).await {
                eprintln!("work: {e}");
            }
            tokio::time::sleep(TICK).await;
        }
    });
    Ok(())
}

/// Fires a queue pass in the background (after enqueueing or cancelling).
pub fn kick(inner: &Arc<Inner>) {
    let inner = inner.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = pump::pump(&inner).await {
            eprintln!("work: {e}");
        }
    });
}
