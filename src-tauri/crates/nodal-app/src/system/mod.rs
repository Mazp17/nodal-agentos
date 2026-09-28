//! System context: legacy data import and host diagnostics. Filled in wave 3a.

use std::sync::Arc;

use crate::core::Core;

/// Filled in wave 3a; holds `core` for `import_legacy` (`spawn_blocking`, `clock`, `env`).
#[allow(dead_code)]
pub struct System {
    core: Arc<Core>,
}

impl System {
    pub(crate) fn new(core: Arc<Core>) -> Self {
        Self { core }
    }
}
