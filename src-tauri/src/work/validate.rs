//! Input validation (projects, repos, tasks, executors, settings). Anything that touches
//! disk is blocking: it's called from `blocking`.
//!
//! Moved to `nodal_domain::board::validate`; re-exported here so current uses don't break.
//! `plan_file` moved to `nodal_host::plans` (nothing in this crate calls it directly anymore:
//! `nodal_app::sources::moved::resolve_moved` now reaches it through the `PlanFiles` port).
//! `root_path` moved to `nodal_app::board::ops` (uses `env.fs.canonical_dir`, board's injected
//! port).

pub use nodal_domain::board::validate::*;
