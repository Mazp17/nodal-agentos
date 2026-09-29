//! Bridge to `nodal-store`'s query modules. Kept only so existing imports keep working;
//! removed in W4.

/// Only the bridge is left; `agent_api::tools` and wave 3c's `Execution` now reach these
/// straight from `nodal_store`.
#[allow(unused_imports)]
pub use nodal_store::board::{projects, repos, tasks};
#[allow(unused_imports)]
pub use nodal_store::execution::runs;

/// Only the bridge is left; nothing in this crate calls it directly anymore
/// (`agent_api::tools` now imports it straight from `nodal_domain::serde_util`).
#[allow(unused_imports)]
pub use nodal_domain::serde_util::double_option;
