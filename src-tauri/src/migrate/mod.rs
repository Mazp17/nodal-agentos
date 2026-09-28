//! The import logic (copy, then import inside a transaction) lives in `nodal_store::legacy`.
//! The command moved to `SRC/commands/system.rs`; re-exported so current uses don't break.

/// Only the bridge is left; nothing in this crate calls it directly anymore.
#[allow(unused_imports)]
pub use crate::commands::system::import_legacy_data;
