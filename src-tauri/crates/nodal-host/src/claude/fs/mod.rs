//! WARNING: Claude Code's internal, UNDOCUMENTED format (observed in v2.1.281).
//!
//! Everything that depends on how Claude Code prints its output or writes its files lives
//! here and only here, so that when it changes there's a single place to touch:
//! - `claude --bg` output (the `backgrounded · <id>` line): `bg_output`;
//! - `claude agents --json --all` output: `agents_json`;
//! - layout of `~/.claude/projects/<slug>/<sessionId>/` (journal, meta, transcripts,
//!   final summary `workflows/wf_*.json`, script `workflows/scripts/*-wf_*.js`): `paths`,
//!   `workflow_detail`, `js_script`, `last_tool`, `transcript`, `usage`, `review_denial`;
//! - reading a session for the app (`readout`).
//!
//! The `-p` stream-json protocol that chats speak lives in `claude::stream_json`, which
//! reuses the content-block helpers of `transcript`.
//!
//! Policy: tolerate anything unknown (new fields, unexpected types, lines cut off
//! mid-write) and degrade to `None` instead of failing.

pub mod agents_json;
pub mod bg_output;
pub mod js_script;
pub mod last_tool;
pub mod paths;
pub mod readout;
pub mod review_denial;
pub mod transcript;
pub mod usage;
pub mod workflow_detail;

use std::io;
use std::path::Path;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

/// Reads a whole file in one pass, tolerating invalid byte sequences (matches this tree's
/// tolerance for odd data: `transcript::read_transcript_text` does the same). Used where a
/// file used to be opened once per field and is now read once and scanned in memory (P12).
pub(crate) fn read_to_string_lossy(path: &Path) -> io::Result<String> {
    std::fs::read(path).map(|b| String::from_utf8_lossy(&b).into_owned())
}

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
