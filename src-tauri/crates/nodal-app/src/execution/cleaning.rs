//! Tasks whose worktree is being deleted. `cleanup_worktree` marks the task and checks that
//! it has no pending runs in the same section while holding the database; `enqueue_work`
//! rejects a marked task in its transaction. That way a launch can't reuse a worktree that is
//! being deleted.
//!
//! Moved from `work::mod` (was `Cleaning`/`CleaningGuard`/`CLEANING_ERR`); the old path bridges
//! here so current uses don't break.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

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
