//! One-line summary of a tool call's input.

use serde_json::Value;

use super::truncate;

pub(crate) fn tool_summary_n(input: &Value, max: usize) -> Option<String> {
    const KEYS: [&str; 9] = [
        "command",
        "file_path",
        "pattern",
        "path",
        "url",
        "query",
        "skill",
        "description",
        "prompt",
    ];
    let s = KEYS.iter().find_map(|k| input.get(*k)?.as_str())?;
    let one_line = s.lines().next().unwrap_or("").trim();
    Some(truncate(one_line, max))
}
