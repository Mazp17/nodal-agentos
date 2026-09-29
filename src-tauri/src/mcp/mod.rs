//! MCP server for agents: create, update and read tasks and runs without the UI.
//!
//! - `adapters::mcp_socket`: the app listens on a Unix socket in its data folder (`0600`, no
//!   network port) and runs each request through `AgentApi::call`, the same `ops`/queries as
//!   the Tauri commands;
//! - `stdio`: the `nodal-mcp` binary speaks MCP (JSON-RPC over stdio) and forwards each
//!   tool call to that socket;
//! - `nodal_app::agent_api::tools`: tool schemas and dispatch;
//! - `commands::mcp`: status, stop and restart from Settings → Diagnostics.
//!
//! Between `nodal-mcp` and the app, one JSON line each way: `{"tool", "arguments"}` →
//! `{"result": …}` or `{"error": "…"}`.

/// Moved to `adapters::mcp_socket`; re-exported so current uses don't break.
pub(crate) mod server;
/// Moved to `nodal_app::agent_api::tools`; re-exported so current uses don't break.
pub(crate) mod tools;

/// Moved to `nodal_mcp_proto::stdio` (W1); still `nodal_lib::mcp::stdio` for the rest of the crate.
pub use nodal_mcp_proto::stdio;

/// Moved to `SRC/commands/mcp.rs`; re-exported so current uses don't break.
pub mod commands {
    pub use crate::commands::mcp::{mcp_restart, mcp_status, mcp_stop, McpState, McpStatus};
}
