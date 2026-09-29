//! "Merge into <base> & done": lands a task branch on the branch its worktree started from
//! without checking anything out in the main repo. All blocking.
//!
//! Moved to `nodal_domain::execution::worktree` (types) and `nodal_host::git::merge`
//! (operations); re-exported here so current uses don't break.

pub use nodal_domain::execution::worktree::MergeReport;
/// `MergeOutcome` is still matched inside `nodal_app::execution::worktrees`, not at this
/// path anymore.
#[allow(unused_imports)]
pub use nodal_domain::execution::worktree::MergeOutcome;
/// Wave 3c's `Execution::merge_worktree` calls `env.git` (the port) directly; nothing in
/// this crate reaches the bridge anymore.
#[allow(unused_imports)]
pub use nodal_host::git::merge::{merge, push_base};
