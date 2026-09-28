//! "Merge into <base> & done": lands a task branch on the branch its worktree started from
//! without checking anything out in the main repo. All blocking.
//!
//! Moved to `nodal_domain::execution::worktree` (types) and `nodal_host::git::merge`
//! (operations); re-exported here so current uses don't break.

pub use nodal_domain::execution::worktree::{MergeOutcome, MergeReport};
pub use nodal_host::git::merge::{merge, push_base};
