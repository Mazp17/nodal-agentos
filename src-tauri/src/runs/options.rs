//! Validation of the per-repo launch flags and their translation into `claude` arguments.
//!
//! Moved to `nodal_domain::execution::options`; re-exported here so current uses don't break.

/// Wave 3c's `Execution` calls `nodal_domain::execution::options` directly; nothing in this
/// crate reaches the bridge anymore.
#[allow(unused_imports)]
pub use nodal_domain::execution::options::*;
