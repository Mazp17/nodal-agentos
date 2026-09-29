//! Moved to `nodal_host::claude::catalog`; re-exported so current uses don't break.

pub use nodal_domain::model::executors::ExecutorInfo;
/// Only the bridge is left; nothing in this crate calls them directly anymore.
#[allow(unused_imports)]
pub use nodal_domain::model::executors::{is_valid_agent_name, AgentDef};
/// Wave 3c's `Execution`/`Board` call `env.claude_config` (the port) directly; nothing in
/// this crate reaches the bridge anymore.
#[allow(unused_imports)]
pub use nodal_host::claude::catalog::*;
