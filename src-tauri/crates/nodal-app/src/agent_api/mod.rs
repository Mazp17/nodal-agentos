//! Agent API context: MCP tool dispatch for `nodal-mcp`. Filled in wave 3c.

use std::sync::Arc;

use crate::core::Core;

/// Filled in wave 3c; holds `core` for `call` (`db.guard()`, `tools::call`, notify per project).
#[allow(dead_code)]
pub struct AgentApi {
    core: Arc<Core>,
}

impl AgentApi {
    pub(crate) fn new(core: Arc<Core>) -> Self {
        Self { core }
    }
}
