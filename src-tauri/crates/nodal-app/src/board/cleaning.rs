//! Tasks whose worktree is being cleaned up or merged: a second cleanup/merge on the same task
//! is rejected while the first one runs.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Default)]
pub struct Cleaning(Arc<Mutex<HashSet<String>>>);

impl Cleaning {
    /// Marks the task; `None` if it's already marked. The mark goes away with the guard.
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
