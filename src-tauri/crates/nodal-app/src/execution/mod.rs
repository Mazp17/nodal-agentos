//! Execution context: the run queue, worktrees, diffs and the background pump. Filled in
//! wave 3c.

use std::sync::Arc;

use crate::core::Core;

/// Filled in wave 3c; holds `core` for enqueue, pump, cancel, worktrees and diff.
#[allow(dead_code)]
pub struct Execution {
    core: Arc<Core>,
}

impl Execution {
    pub(crate) fn new(core: Arc<Core>) -> Arc<Self> {
        Arc::new(Self { core })
    }
}
