//! Execution context: the run queue, worktrees, diffs and the background pump.
//!
//! - `enqueue`: prompt/executor/worktree resolution and inserting a run (was `work::launch`);
//! - `pump`: the periodic pass that detects finishes, applies transitions and launches (was
//!   `work::pump`), plus the queue-pass lock and `App::run_pump`/`kick`'s single pass;
//! - `cancel`: dequeuing/stopping a run;
//! - `worktrees`: status, cleanup and merge;
//! - `summary`: the queue's summary for the UI;
//! - `diff`: a run's diff and its working folder;
//! - `cleaning`: tasks whose worktree is being deleted (was `work::Cleaning`).
//!
//! Moved from `work::{mod, launch, pump}` plus the run-related commands in
//! `commands::execution`. All I/O goes through `core.claude`, `core.sessions`, `env.git`,
//! `env.plans`, `env.fs`, `env.claude_config`, `core.clock` and `core.notifier` — nodal-app
//! can't depend on nodal-host or touch the filesystem/processes directly (clippy.toml).

mod cancel;
pub mod cleaning;
mod diff;
mod enqueue;
mod pump;
mod summary;
mod worktrees;

use std::sync::{Arc, Mutex, Weak};

use nodal_domain::model::events::ChangeKind;
use nodal_domain::model::{Run, RunLight};
use nodal_domain::util::check_id;
use nodal_store::execution::runs as qruns;
use nodal_store::{Conn as Connection, StoreError};

use crate::core::{AppError, Core};

/// Note left on a run that was still `launching` when the app closed (`setup`'s first step,
/// today's `work::init`). Kept `pub` (unlike the rest of `pump`): the shell passes it to
/// `fail_stale_launches`.
pub use pump::NOTE_APP_CLOSED;

/// What changes when a run is enqueued, launched or cancelled.
const RUN_KINDS: &[ChangeKind] = &[ChangeKind::Runs, ChangeKind::Queue, ChangeKind::Tasks];

fn opt_id(id: Option<String>, what: &str) -> Result<Option<String>, String> {
    let id = id.filter(|s| !s.trim().is_empty());
    if let Some(i) = &id {
        check_id(i, what)?;
    }
    Ok(id)
}

/// The run queue, worktrees, diffs and the background pump. Holds the queue-pass lock, the
/// last pass' error (for `work_summary`) and which tasks' worktrees are being cleaned up (was
/// `work::Inner`).
pub struct Execution {
    core: Arc<Core>,
    /// Serializes queue passes: only one at a time decides and launches.
    pump: tokio::sync::Mutex<()>,
    /// Error of the last queue pass (`None` if it went fine), for `work_summary`.
    pump_error: Mutex<Option<String>>,
    /// Tasks whose worktree is being cleaned up.
    cleaning: cleaning::Cleaning,
    /// Lets `&self` facade methods (Tauri commands) spawn a detached queue pass on `core.rt`
    /// (`kick`), which needs an owned `Arc<Execution>` for the spawned future.
    self_ref: Weak<Execution>,
}

impl Execution {
    pub(crate) fn new(core: Arc<Core>) -> Arc<Self> {
        Arc::new_cyclic(|weak| Self {
            core,
            pump: tokio::sync::Mutex::new(()),
            pump_error: Mutex::new(None),
            cleaning: cleaning::Cleaning::default(),
            self_ref: weak.clone(),
        })
    }

    /// Runs `f` with the connection; `ops` errors already come ready to display.
    async fn db<T, F>(&self, f: F) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, String> + Send + 'static,
    {
        Ok(nodal_store::with_db(&self.core.db, move |c| f(c).map_err(StoreError::Invalid)).await?)
    }

    /// `project_id` (optional) filters by project in addition to task.
    pub async fn list_task_runs(&self, task_id: Option<String>, project_id: Option<String>) -> Result<Vec<Run>, AppError> {
        let task_id = opt_id(task_id, "task")?;
        let project_id = opt_id(project_id, "project")?;
        self.db(move |c| Ok(qruns::list_filtered(c, project_id.as_deref(), task_id.as_deref())?)).await
    }

    /// Like `list_task_runs`, without `prompt` or `extraInstructions`.
    pub async fn list_runs_light(&self, project_id: Option<String>, task_id: Option<String>) -> Result<Vec<RunLight>, AppError> {
        let task_id = opt_id(task_id, "task")?;
        let project_id = opt_id(project_id, "project")?;
        let runs = self.db(move |c| Ok(qruns::list_filtered(c, project_id.as_deref(), task_id.as_deref())?)).await?;
        Ok(runs.into_iter().map(RunLight::from).collect())
    }

    pub async fn get_run(&self, run_id: String) -> Result<Run, AppError> {
        check_id(&run_id, "run")?;
        self.db(move |c| Ok(qruns::get(c, &run_id)?)).await
    }

    /// The last run of each task (no history limit), lightweight.
    pub async fn latest_runs_by_task(&self, project_id: Option<String>) -> Result<Vec<RunLight>, AppError> {
        let project_id = opt_id(project_id, "project")?;
        let runs = self.db(move |c| Ok(qruns::latest_by_task(c, project_id.as_deref())?)).await?;
        Ok(runs.into_iter().map(RunLight::from).collect())
    }

    pub async fn list_queue(&self) -> Result<Vec<Run>, AppError> {
        self.db(|c| Ok(qruns::queue(c)?)).await
    }

    pub async fn reorder_queue(&self, run_ids: Vec<String>) -> Result<(), AppError> {
        for id in &run_ids {
            check_id(id, "run")?;
        }
        self.db(move |c| Ok(qruns::reorder_queue(c, &run_ids)?)).await?;
        self.core.notifier.notify(ChangeKind::Queue, None);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
