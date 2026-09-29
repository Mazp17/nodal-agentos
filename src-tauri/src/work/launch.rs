//! Run building: executor and option resolution (run → task → repo → project), worktree,
//! prompt per executor and launch flags.
//!
//! Constants, resolution and prompt building moved to `nodal_domain::execution::prompts`;
//! `test_commands` moved to `nodal_host::repo`; both re-exported here so current uses don't
//! break. Everything else (`enqueue_work`, `enqueue_review`, `confirm_legacy`, `build_review`
//! and their helpers) moved to `nodal_app::execution::enqueue` (wave 3c); nothing in this
//! crate calls it at this path anymore.

/// Only `runs::tests::real_launch_with_append_system_prompt` (ignored) still reaches
/// `UNATTENDED_SYSTEM_PROMPT` through this path; rustc's unused-import check doesn't credit
/// that for a glob re-export.
#[allow(unused_imports)]
pub use nodal_domain::execution::prompts::*;

/// Moved to `nodal_host::repo::test_commands`; re-exported so current uses don't break.
/// Wave 3c's `Execution::pump` calls `env.fs.test_commands` (the port) directly; nothing in
/// this crate reaches the bridge anymore.
#[allow(unused_imports)]
pub use nodal_host::repo::test_commands;
