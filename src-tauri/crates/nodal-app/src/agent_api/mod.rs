//! Agent API context: MCP tool dispatch for `nodal-mcp`. One request: the same `ops` as the
//! UI with the database locked, then the same `nodal://changed` the Tauri commands emit (was
//! `mcp::server::respond`). `start_task_chat` goes through the async `Chats` facade instead.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use nodal_domain::model::events::ChangeKind;
use serde_json::Value;

use crate::core::Core;
use crate::chats::Chats;

pub mod tools;

const CHAT_TOOL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(25);

pub struct AgentApi {
    core: Arc<Core>,
    chats: Chats,
}

impl AgentApi {
    pub(crate) fn new(core: Arc<Core>) -> Self {
        Self { chats: Chats::new(core.clone()), core }
    }

    /// Drives a `Chats` call to completion. `call` runs on the socket's own connection
    /// threads, never on the runtime, where blocking would panic.
    /// Gives up before `nodal-mcp`'s own 30s socket timeout, so the agent gets a clear error.
    fn block_on<F: Future>(&self, fut: F) -> Result<F::Output, String> {
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err("Internal error: chat tools can't be called from the async runtime.".into());
        }
        self.core
            .rt
            .block_on(async { tokio::time::timeout(CHAT_TOOL_TIMEOUT, fut).await })
            .map_err(|_| {
                "Nodal took too long to answer; it may still be doing it. Check the Nodal app."
                    .to_string()
            })
    }

    /// Opens (or returns) the task's chat; the UI picks it up through the `Tasks` notification.
    fn start_task_chat(&self, args: Value) -> Result<Value, String> {
        let task = args.get("task").and_then(Value::as_str).ok_or("Pass `task`.")?;
        let task = tools::resolve_task(&self.core.db.guard(), task)?;
        let chat = self
            .block_on(self.chats.start_task_chat(task.id))?
            .map_err(|e| e.to_string())?;
        Ok(serde_json::json!({"chatId": chat.id, "title": chat.title, "taskId": chat.task_id}))
    }

    /// Where the MCP socket adapter binds `mcp.sock` (`Env::data_dir`).
    pub fn data_dir(&self) -> &Path {
        &self.core.env.data_dir
    }

    /// Runs one MCP tool call with the database locked, then notifies `Tasks` for the
    /// project the tool changed, if any.
    pub fn call(&self, tool: &str, args: Value) -> Result<Value, String> {
        if tool == "start_task_chat" {
            return self.start_task_chat(args);
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
