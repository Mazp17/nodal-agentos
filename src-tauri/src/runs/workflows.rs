//! Moved to `nodal_host::claude::workflows`; re-exported so current uses don't break.
//! Only the bridge is left; nothing in this crate calls it directly anymore (the catalog
//! that used it, `work::executors`, now bridges straight to `nodal_host::claude::catalog`).

#[allow(unused_imports)]
pub use nodal_domain::model::executors::is_valid_workflow_name;
#[allow(unused_imports)]
pub use nodal_host::claude::workflows::*;
