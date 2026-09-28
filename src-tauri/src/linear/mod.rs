//! Linear in read-only mode. `LinearState` and `linear_issue_detail` moved to
//! `SRC/commands/linear.rs`; re-exported so current uses don't break. The client, model and
//! error types live in `nodal_linear`.

/// Only the bridge is left; nothing in this crate calls it directly anymore.
#[allow(unused_imports)]
pub use nodal_linear::LinearError;

pub use crate::commands::linear::LinearState;
/// Only the bridge is left; nothing in this crate calls it directly anymore.
#[allow(unused_imports)]
pub use crate::commands::linear::linear_issue_detail;
