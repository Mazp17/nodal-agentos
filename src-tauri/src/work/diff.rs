//! A run's diff: `git diff <base>...HEAD` in its folder, plus anything uncommitted
//! (including new untracked files), parsed into files and hunks.
//!
//! Types and parsing moved to `nodal_domain::diff`; git operations moved to
//! `nodal_host::git::diff`; both re-exported here so current uses don't break.

pub use nodal_domain::diff::*;
/// Wave 3c's `Execution::{run_diff,cancel_run}` call `env.git` (the port) directly; nothing
/// in this crate reaches the bridge anymore.
#[allow(unused_imports)]
pub use nodal_host::git::diff::{collect, collect_branch, commits, current_branch};
