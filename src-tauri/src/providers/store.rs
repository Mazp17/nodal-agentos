//! SQL for source links, linked tasks, outbox and the active-runs query moved to
//! `nodal_store::sources`; re-exported so current uses don't break (e.g.
//! `commands::execution`'s `moved_ids` check). `resolve_moved`/`MovedAction` moved to
//! `nodal_app::sources::moved`.

/// Wave 3c's `Execution::work_summary` now calls `nodal_store::sources::moved_ids` directly;
/// nothing in this crate reaches the bridge anymore.
#[allow(unused_imports)]
pub use nodal_store::sources::*;
