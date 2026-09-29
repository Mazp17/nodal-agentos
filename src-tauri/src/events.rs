//! `Kind`/`Changed` moved to `nodal_domain::model::events`; `Events` moved to
//! `SRC/adapters/events.rs`, where it implements `ChangeNotifier`. Re-exported so current
//! uses don't break.

/// Only the bridge is left; `mcp::server` (agent_api) and wave 3c's `Execution` now reach
/// `ChangeKind` straight from `nodal_domain`.
#[allow(unused_imports)]
pub use nodal_domain::model::events::ChangeKind as Kind;
/// Only the bridge is left; nothing in this crate calls them directly anymore.
#[allow(unused_imports)]
pub use nodal_domain::model::events::{Changed, CHANGED};

/// Only the bridge is left; every remaining caller (`lib.rs`, `commands::chats`,
/// `adapters::mcp_socket`'s tests) now names `crate::adapters::events::Events` directly —
/// wave 3c's `commands::execution::setup` was the last one going through this path, to build
/// the legacy `work::Inner`'s `events` field, and that's gone now too.
#[allow(unused_imports)]
pub use crate::adapters::events::Events;
