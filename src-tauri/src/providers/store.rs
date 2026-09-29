//! SQL for source links, linked tasks, outbox and the active-runs query moved to
//! `nodal_store::sources`; re-exported so current uses don't break (e.g.
//! `commands::execution`'s `moved_ids` check). `resolve_moved`/`MovedAction` moved to
//! `nodal_app::sources::moved`.

pub use nodal_store::sources::*;
