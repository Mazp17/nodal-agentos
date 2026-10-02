//! Agent API context: MCP tool dispatch for `nodal-mcp`. One request: the same `ops` as the
//! UI with the database locked, then the same `nodal://changed` the Tauri commands emit (was
//! `mcp::server::respond`). Run tools (`runs`) go through the `Execution` facade instead.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use nodal_domain::model::events::ChangeKind;
use serde_json::Value;

use crate::core::Core;
use crate::execution::Execution;

mod runs;
pub mod tools;

const RUN_TOOL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(25);

pub struct AgentApi {
    core: Arc<Core>,
    execution: Arc<Execution>,
}

impl AgentApi {
    pub(crate) fn new(core: Arc<Core>, execution: Arc<Execution>) -> Self {
        Self { core, execution }
    }

    /// Drives an `Execution` call to completion. `call` runs on the socket's own connection
    /// threads, never on the runtime, where blocking would panic.
    /// Gives up before `nodal-mcp`'s own 30s socket timeout, so the agent gets a clear error.
    fn block_on<F: Future>(&self, fut: F) -> Result<F::Output, String> {
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err("Internal error: run tools can't be called from the async runtime.".into());
        }
        self.core
            .rt
            .block_on(async { tokio::time::timeout(RUN_TOOL_TIMEOUT, fut).await })
            .map_err(|_| {
                "Nodal took too long to answer; it may still be doing it. Check with get_run or get_queue."
                    .to_string()
            })
    }

    /// Where the MCP socket adapter binds `mcp.sock` (`Env::data_dir`).
    pub fn data_dir(&self) -> &Path {
        &self.core.env.data_dir
    }

    /// Runs one MCP tool call with the database locked, then notifies `Tasks` for the
    /// project the tool changed, if any.
    pub fn call(&self, tool: &str, args: Value) -> Result<Value, String> {
        if runs::TOOLS.contains(&tool) {
            let args = if args.is_null() { serde_json::json!({}) } else { args };
            return runs::call(self, tool, args);
        }
        // The lock is released before notifying, as before.
        let outcome = {
            let mut conn = self.core.db.guard();
            let now = self.core.clock.now_ms();
            tools::call(&mut conn, &self.core.env, tool, args, now)?
        };
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
