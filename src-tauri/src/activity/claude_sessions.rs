//! Moved to `nodal_host::claude::activity_files`; re-exported so current uses don't break.
//! Only the bridge is left; nothing in this crate calls it directly anymore.

#[allow(unused_imports)]
pub use nodal_domain::model::activity::AgentSession;
#[allow(unused_imports)]
pub use nodal_host::claude::activity_files::*;
