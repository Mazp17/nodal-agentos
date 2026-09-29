//! Adapters wiring `nodal_domain::ports` (and other cross-crate glue) to Tauri: the shell's
//! side of the ports nodal-app depends on.

pub mod chat_sink;
pub mod events;
pub mod mcp_socket;
