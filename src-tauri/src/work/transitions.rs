//! Task and run state transitions (pure). The queue gathers the data (`claude agents`
//! output, session files, executor catalog) and applies what's decided here in a single
//! transaction.
//!
//! Moved to `nodal_domain::execution::transitions`; re-exported here so current uses don't
//! break.

pub use nodal_domain::execution::transitions::*;
