//! Test data builders shared by the `work` tests.
//!
//! Moved to `nodal_domain::testutil`; re-exported here so current uses don't break.

/// Only the bridge is left; every remaining test builder in this crate has moved to
/// nodal-app/nodal-host and reaches `nodal_domain::testutil` directly.
#[allow(unused_imports)]
pub use nodal_domain::testutil::*;
