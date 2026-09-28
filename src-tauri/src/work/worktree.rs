//! Task worktrees: `~/.nodal/worktrees/<repo>/<task-slug>` with the branch
//! `nodal/<task-slug>`, created from the repo's current branch before the first run and
//! reused by the following ones (handoffs included). All blocking.
//!
//! Naming and status types moved to `nodal_domain::execution::worktree`; git operations
//! moved to `nodal_host::git::worktree`; both re-exported here so current uses don't break.

pub use nodal_domain::execution::worktree::*;
pub use nodal_host::git::worktree::{cleanup, current_base, ensure, status};
