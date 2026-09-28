//! Moved to `nodal_host::git`; re-exported so current uses don't break.
//! `git_version` moved to `SRC/commands/host.rs`.

/// Only the bridge is left; nothing in this crate calls them directly anymore.
#[allow(unused_imports)]
pub use nodal_host::git::{ok, run, toplevel, GitOutput, GIT_TIMEOUT};

/// Only the bridge is left; nothing in this crate calls it directly anymore.
#[allow(unused_imports)]
pub use crate::commands::host::git_version;
