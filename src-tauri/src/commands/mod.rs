//! Tauri commands, one file per bounded context.
//!
//! Most bodies take `State<'_, Arc<App>>` and reject with `CommandError` (from an `AppError`
//! or a bare `String`); a few contexts with no shared `App` state of their own (`host`,
//! `linear`, `mcp`, `pty`) still take their own state, or none, and reject with a plain
//! `String`.

pub mod board;
pub mod chats;
pub mod execution;
pub mod host;
pub mod linear;
pub mod mcp;
pub mod pty;
pub mod sessions;
pub mod sources;
pub mod system;

use serde::{Serialize, Serializer};

/// What a Tauri command rejects with, once its context returns `AppError` instead of a bare
/// `String`. Serializes exactly like today's `Result<T, String>` (the frontend reads
/// `error` as a plain string), so switching a command over is invisible to the UI.
#[derive(Debug)]
pub struct CommandError(String);

impl Serialize for CommandError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl From<nodal_app::AppError> for CommandError {
    fn from(e: nodal_app::AppError) -> Self {
        CommandError(e.to_string())
    }
}

impl From<String> for CommandError {
    fn from(e: String) -> Self {
        CommandError(e)
    }
}

/// Runs `f` on a blocking thread (disk, git) without stalling the runtime. Shell-only: it
/// wraps `tauri::async_runtime::spawn_blocking`, which nodal-app/nodal-domain can't depend on.
pub(crate) async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| format!("Internal error: {e}"))?
}

#[cfg(test)]
mod tests;
