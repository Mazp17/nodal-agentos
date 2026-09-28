//! Bridge to `nodal-store`'s query modules. Kept only so existing imports keep working;
//! removed in W4.

pub use nodal_store::board::{projects, repos, tasks};
pub use nodal_store::chats;
pub use nodal_store::execution::runs;

/// Moved to `nodal_domain::serde_util`; re-exported so current uses don't break.
pub use nodal_domain::serde_util::double_option;
