//! Last `tool_use` of an agent transcript's tail.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde_json::Value;

use super::truncate;

/// Last `tool_use` of an `agent-<id>.jsonl` transcript. Reads only the file's tail
/// (they can weigh MBs): first 64 KB, and if there's none there, 1 MB.
pub fn last_tool_from_transcript(path: &Path) -> Option<(String, Option<String>)> {
    let mut file = fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    for window in [64 * 1024u64, 1024 * 1024] {
        let start = len.saturating_sub(window);
        file.seek(SeekFrom::Start(start)).ok()?;
        let mut buf = Vec::new();
        file.by_ref().take(window).read_to_end(&mut buf).ok()?;
        let text = String::from_utf8_lossy(&buf);
        // If we didn't start at the beginning, the first line is cut off.
        let text = if start > 0 {
            text.split_once('\n').map_or("", |(_, r)| r)
        } else {
            &text
        };
        if let Some(found) = last_tool_in_lines(text) {
            return Some(found);
        }
        if start == 0 {
            break;
        }
    }
    None
}

pub fn last_tool_in_lines(text: &str) -> Option<(String, Option<String>)> {
    text.lines().rev().find_map(|line| {
        // Cheap filter before parsing lines that can be huge.
        if !line.contains("\"tool_use\"") {
            return None;
        }
        let v: Value = serde_json::from_str(line).ok()?;
        if v.get("type")?.as_str()? != "assistant" {
            return None;
        }
        let content = v.get("message")?.get("content")?.as_array()?;
        content.iter().rev().find_map(|c| {
            if c.get("type")?.as_str()? != "tool_use" {
                return None;
            }
            let name = c.get("name")?.as_str()?.to_string();
            Some((name, c.get("input").and_then(tool_summary)))
        })
    })
}

pub(crate) fn tool_summary(input: &Value) -> Option<String> {
    tool_summary_n(input, 80)
}

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
