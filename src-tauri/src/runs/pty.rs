//! Moved to `nodal_host::pty`; re-exported so current uses don't break. The `Channel`/
//! `Webview` glue (this crate has tauri; the host crate doesn't) moved to
//! `SRC/commands/pty.rs`, wrapped into the plain callbacks `PtySessions::attach` takes.

pub use nodal_host::pty::PtySessions;

/// Only the bridge is left; nothing in this crate calls them directly anymore.
#[allow(unused_imports)]
pub use crate::commands::pty::{pty_attach, pty_close, pty_resize, pty_write};
