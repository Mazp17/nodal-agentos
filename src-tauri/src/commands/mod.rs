//! Tauri commands, one file per bounded context (frozen split: wave 3 fills each context's
//! use cases behind it, one agent per file, without touching this one).
//!
//! Command bodies still validate ids, run the legacy `WorkState`/`ChatState`/`Db` and reject
//! with `String`, exactly like before the move: only their location and imports changed.
//! `CommandError` is ready for wave 3, which switches each context to `State<Arc<App>>` and
//! `AppError`.

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
