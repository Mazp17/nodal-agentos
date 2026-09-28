//! Moved to `nodal_host::claude::trust`; re-exported so current uses don't break.
//! `repo_trust` moved to `SRC/commands/host.rs`.

pub use nodal_host::claude::trust::repo_trust_blocking;
/// Only the bridge is left; nothing in this crate calls them directly anymore.
#[allow(unused_imports)]
pub use nodal_host::claude::trust::{global_config_file, trust_of, RepoTrust, TrustSource};

/// Only the bridge is left; nothing in this crate calls it directly anymore.
#[allow(unused_imports)]
pub use crate::commands::host::repo_trust;
