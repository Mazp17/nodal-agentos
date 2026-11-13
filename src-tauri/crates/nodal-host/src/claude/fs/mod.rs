//! WARNING: Claude Code's internal, UNDOCUMENTED format (observed in v2.1.281).
//!
//! Everything that depends on how Claude Code prints its output or writes its files lives
//! here and only here, so that when it changes there's a single place to touch:
//! - layout of `~/.claude/projects/<slug>/<sessionId>.jsonl` and its transcript format:
//!   `paths`, `last_tool`, `transcript`;
//!
//! The `-p` stream-json protocol that chats speak lives in `claude::stream_json`, which
//! reuses the content-block helpers of `transcript`.
//!
//! Policy: tolerate anything unknown (new fields, unexpected types, lines cut off
//! mid-write) and degrade to `None` instead of failing.

pub mod last_tool;
pub mod paths;
pub mod transcript;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

/// Deserializes an optional field without failing if the type isn't the expected one.
pub(crate) fn lenient<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let v = Value::deserialize(d)?;
    Ok(serde_json::from_value(v).ok())
}

/// Moved to `nodal_domain::sessions::transcript`; kept local under the historic name used
/// throughout this module tree.
pub(crate) fn truncate(s: &str, max: usize) -> String {
    nodal_domain::sessions::transcript::clip(s, max).0
}
