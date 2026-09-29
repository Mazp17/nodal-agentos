//! Task and run state transitions (pure). The queue gathers the data (`claude agents`
//! output, session files, executor catalog) and applies what's decided here in a single
//! transaction.
//!
//! Moved to `nodal_domain::execution::transitions`; re-exported here so current uses don't
//! break.

/// Wave 3c's `Execution::{pump,cancel_run}` call `nodal_domain::execution::transitions`
/// directly; nothing in this crate reaches the bridge anymore.
#[allow(unused_imports)]
pub use nodal_domain::execution::transitions::*;
