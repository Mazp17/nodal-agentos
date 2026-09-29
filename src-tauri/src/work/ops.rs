//! Moved to `nodal_app::board::ops`; re-exported so current uses don't break.

/// Only the bridge is left; `agent_api::AgentApi` and wave 3c's `Execution` now call
/// `nodal_app::board::ops` directly.
#[allow(unused_imports)]
pub use nodal_app::board::ops::*;
