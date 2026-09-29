//! Paths: `~`, the canonical git root and Nodal's folder in the home directory.
//!
//! `DEV` and `data_dir` moved to `SRC/paths.rs`; `resolve_git_root` and `scan_git_repos`
//! moved to `SRC/commands/host.rs`. Both are re-exported so current uses don't break.

/// Moved to `nodal_mcp_proto::paths` (W1), which keeps its own copy of `DEV` (this crate's
/// `DEV` below stays, since it's used well beyond paths: `updates.rs`, `secrets.rs`, `lib.rs`).
pub use nodal_mcp_proto::paths::mcp_socket;
/// Only the bridge is left; nothing in this crate calls it directly anymore (`nodal_home`
/// and the other host-side path functions now use `nodal_host::paths::home` instead).
#[allow(unused_imports)]
pub use nodal_mcp_proto::paths::home;
/// Only their old-path bridge is left; nothing in this crate calls them once `mcp::stdio`'s
/// standalone lookup and its test moved to nodal-mcp-proto with it.
#[allow(unused_imports)]
pub use nodal_mcp_proto::paths::{data_dir_standalone, APP_IDENTIFIER};

/// Moved to `nodal_host::paths`; re-exported so current uses don't break.
pub use nodal_host::paths::{git_repos_under, git_root_of, nodal_home};
/// Only the bridge is left; nothing in this crate calls it directly anymore
/// (`agent_api::tools::resolve_repo` now reaches it through `Env::fs`, its injected port).
#[allow(unused_imports)]
pub use nodal_host::paths::expand_home;

/// Only the bridge is left; nothing in this crate calls them directly anymore.
#[allow(unused_imports)]
pub use crate::commands::host::{resolve_git_root, scan_git_repos};
pub use crate::paths::{data_dir, DEV};

#[cfg(test)]
pub(crate) mod tests;
