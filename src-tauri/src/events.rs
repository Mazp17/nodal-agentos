//! `Kind`/`Changed` moved to `nodal_domain::model::events`; `Events` moved to
//! `SRC/adapters/events.rs`, where it implements `ChangeNotifier`. Re-exported so current
//! uses don't break.

pub use nodal_domain::model::events::ChangeKind as Kind;
/// Only the bridge is left; nothing in this crate calls them directly anymore.
#[allow(unused_imports)]
pub use nodal_domain::model::events::{Changed, CHANGED};

pub use crate::adapters::events::Events;
