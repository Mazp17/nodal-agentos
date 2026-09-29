//! Work: projects, repos and tasks (CRUD), the single run queue, executors,
//! worktrees, automatic review and diff.
//!
//! - `ops`: synchronous CRUD over the database (commands run it with `with_db`);
//! - `launch`: prompt building and run enqueueing;
//! - `queue` and `transitions`: pure queue and state logic;
//! - `pump`: the periodic pass that detects finishes, applies transitions and launches;
//! - `commands`: the Tauri commands.
//!
//! `Inner`/`WorkState`/`Cleaning`/`init`/`kick` (the pre-wave-3c queue supervisor) are gone:
//! their logic moved to `nodal_app::execution` and their last readers outside this crate
//! (`mcp::server`/`commands::mcp`) moved to `Arc<App>` (agent_api, merged concurrently). See
//! `commands::execution` and `nodal_app::execution` for what replaced them.

pub mod commands;
pub mod diff;
pub mod dto;
pub mod executors;
pub mod launch;
pub mod merge;
pub mod ops;
pub mod pump;
pub mod queue;
pub mod report;
pub mod transitions;
pub mod validate;
pub mod worktree;

#[cfg(test)]
pub(crate) mod testutil;

/// Moved to `nodal_app::Env` (same field names and methods, plus the ports the old code
/// reached directly); re-exported so current uses don't break. Nothing in this crate reaches
/// it at this path anymore: every context builds/holds its own `nodal_app::Env`.
#[allow(unused_imports)]
pub use nodal_app::Env;
