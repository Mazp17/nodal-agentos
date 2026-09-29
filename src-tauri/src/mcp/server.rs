//! Moved to `adapters::mcp_socket`; re-exported so current uses don't break.

/// Only the bridge is left; nothing in this crate calls it directly anymore
/// (`commands::mcp` now reaches it through `adapters::mcp_socket` directly).
#[allow(unused_imports)]
pub use crate::adapters::mcp_socket::{start, Listening};
