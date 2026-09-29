//! Moved to `nodal_app::agent_api::tools`; re-exported so current uses don't break.

/// Only the bridge is left; nothing in this crate calls it directly anymore
/// (`AgentApi::call` reaches it directly, from inside `nodal-app`).
#[allow(unused_imports)]
pub use nodal_app::agent_api::tools::*;
