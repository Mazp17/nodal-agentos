//! Claude Code activity in a repo. `assemble`, `external_sessions_of`, `existing_roots` and
//! `list_agents` moved to `nodal_host::claude::activity`; re-exported so current uses don't
//! break. `external_sessions` moved to `SRC/commands/sessions.rs`.

mod claude_sessions;

/// Moved to `nodal_domain::model::activity`; re-exported so current uses don't break.
pub use nodal_domain::model::activity::{AppRuns, ExternalSessions};
/// Only the bridge is left; nothing in this crate calls them directly anymore.
#[allow(unused_imports)]
pub use nodal_domain::model::activity::{RepoActivity, RepoSessions, SessionActivity, SubagentActivity};
pub use nodal_host::claude::activity::{existing_roots, external_sessions_of, list_agents};
#[allow(unused_imports)]
pub use nodal_host::claude::activity::assemble;

/// Only the bridge is left; nothing in this crate calls it directly anymore.
#[allow(unused_imports)]
pub use crate::commands::sessions::external_sessions;
