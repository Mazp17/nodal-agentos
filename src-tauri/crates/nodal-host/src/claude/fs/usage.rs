//! Tokens spent in a session, computed from the transcript's `usage` fields.

use std::fs;
use std::path::Path;

use serde_json::Value;

/// Tokens of a session transcript: `input + output + cache_creation + cache_read` of
/// each assistant response. Claude Code repeats the `usage` on every line of the same
/// message (one per block), so it's counted once per `message.id`. Includes the
/// sidechains (the session's subagents): they're also the run's spend. `None` if no usage.
pub fn usage_tokens_in<'a>(lines: impl IntoIterator<Item = &'a str>) -> Option<i64> {
    let mut seen = std::collections::HashSet::new();
    let mut total: Option<i64> = None;
    for line in lines {
        if !line.contains("\"usage\"") || !line.contains("\"assistant\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if v.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(msg) = v.get("message") else { continue };
        let Some(usage) = msg.get("usage").filter(|u| u.is_object()) else { continue };
        if let Some(id) = msg.get("id").and_then(Value::as_str) {
            if !seen.insert(id.to_string()) {
                continue;
            }
        }
        let n: i64 = ["input_tokens", "output_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"]
            .iter()
            .filter_map(|k| usage.get(*k).and_then(Value::as_i64))
            .sum();
        total = Some(total.unwrap_or(0) + n);
    }
    total
}

/// `usage_tokens_in` over the whole file, line by line (without loading it into memory).
pub fn read_usage_tokens(path: &Path) -> Option<i64> {
    use std::io::BufRead;
    let file = fs::File::open(path).ok()?;
    let lines: Vec<String> = std::io::BufReader::new(file).lines().map_while(Result::ok).filter(|l| l.contains("\"usage\"")).collect();
    usage_tokens_in(lines.iter().map(String::as_str))
}

#[cfg(test)]
mod tests;
