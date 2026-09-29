//! Bridge to `nodal-store`'s query modules. Kept only so existing imports keep working;
//! removed in W4.

pub use nodal_store::board::{projects, repos, tasks};
pub use nodal_store::execution::runs;

/// Only the bridge is left; nothing in this crate calls it directly anymore
/// (`agent_api::tools` now imports it straight from `nodal_domain::serde_util`).
#[allow(unused_imports)]
pub use nodal_domain::serde_util::double_option;
