//! Wire protocol: one JSON line each way between `nodal-mcp` and the app.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Upper bound of one request line (a plan is at most 512 KB, escaped as JSON).
pub const MAX_LINE: u64 = 4 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub tool: String,
    #[serde(default)]
    pub arguments: Value,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Reply {
    Result(Value),
    Error(String),
}
