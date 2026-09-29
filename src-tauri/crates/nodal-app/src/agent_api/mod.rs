//! Agent API context: MCP tool dispatch for `nodal-mcp`. One request: the same `ops` as the
//! UI with the database locked, then the same `nodal://changed` the Tauri commands emit (was
//! `mcp::server::respond`).

use std::path::Path;
use std::sync::Arc;

use nodal_domain::model::events::ChangeKind;
use serde_json::Value;

use crate::core::Core;

pub mod tools;

pub struct AgentApi {
    core: Arc<Core>,
}

impl AgentApi {
    pub(crate) fn new(core: Arc<Core>) -> Self {
        Self { core }
    }

    /// Where the MCP socket adapter binds `mcp.sock` (`Env::data_dir`).
    pub fn data_dir(&self) -> &Path {
        &self.core.env.data_dir
    }

    /// Runs one MCP tool call with the database locked, then notifies `Tasks` for the
    /// project the tool changed, if any.
    pub fn call(&self, tool: &str, args: Value) -> Result<Value, String> {
        let mut conn = self.core.db.guard();
        let now = self.core.clock.now_ms();
        let outcome = tools::call(&mut conn, &self.core.env, tool, args, now)?;
        if let Some(project_id) = &outcome.changed {
            self.core
                .notifier
                .notify(ChangeKind::Tasks, Some(project_id));
        }
        Ok(outcome.value)
    }
}

#[cfg(test)]
mod tests;
