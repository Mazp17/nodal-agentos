//! Moved to `nodal_host::claude::stream_json`; re-exported so current uses don't break.
//! Only the bridge is left; nothing in this crate calls it directly anymore (the machinery
//! that used it, `chats::process`, now lives entirely in `nodal_host::claude::chats`).

#[allow(unused_imports)]
pub use nodal_domain::model::chat::{PermissionRequest, StreamEvent};
#[allow(unused_imports)]
pub use nodal_host::claude::stream_json::*;
