//! Board context: projects, repos, tasks, relations, executors and settings.

pub mod ops;

use std::sync::Arc;

use crate::core::Core;

/// Filled in wave 3a; holds `core` for the methods it doesn't have yet.
#[allow(dead_code)]
pub struct Board {
    core: Arc<Core>,
}

impl Board {
    pub(crate) fn new(core: Arc<Core>) -> Self {
        Self { core }
    }
}
