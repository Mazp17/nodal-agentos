//! Moved to `nodal_host::{claude::cli, terminal}`; re-exported so current uses don't break.
//! `attach_run`/`open_terminal_at` moved to `SRC/commands/host.rs`.

pub use nodal_domain::execution::worktree::is_valid_run_id;
pub use nodal_host::claude::cli::stop;

/// Only the bridge is left; nothing in this crate calls them directly anymore.
#[allow(unused_imports)]
pub use crate::commands::host::{attach_run, open_terminal_at};
