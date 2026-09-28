//! The MCP Unix-socket server, moved verbatim from `mcp::server` (holding `Arc<App>`
//! instead of `Arc<work::Inner>`). Filled in wave 3c (agent_api); `mcp::server` still
//! backs `commands::mcp::McpState` until then.
