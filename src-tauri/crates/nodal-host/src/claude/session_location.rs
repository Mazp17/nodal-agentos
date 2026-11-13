//! `(cwd, gitBranch)` of a Claude Code session, read from the tail of its transcript
//! (UNDOCUMENTED format: every line may carry `cwd` and `gitBranch`). Transcripts weigh
//! MBs: only the tail is read. Tolerates anything unknown and degrades to `None`.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde_json::Value;

const TAIL_WINDOW: u64 = 256 * 1024;

/// Last `window` bytes of the file, without the first line if it was cut off.
fn read_tail(path: &Path, window: u64) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(window);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    file.take(window).read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    Some(if start > 0 {
        text.split_once('\n').map_or(String::new(), |(_, r)| r.to_string())
    } else {
        text
    })
}

/// The most recent `cwd` and `gitBranch` found in `text`'s JSONL lines.
fn latest_location(text: &str) -> (Option<String>, Option<String>) {
    let (mut cwd, mut branch) = (None, None);
    for line in text.lines().rev() {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { continue };
        if cwd.is_none() {
            cwd = v.get("cwd").and_then(Value::as_str).map(String::from);
        }
        if branch.is_none() {
            branch = v.get("gitBranch").and_then(Value::as_str).map(String::from);
        }
        if cwd.is_some() && branch.is_some() {
            break;
        }
    }
    (cwd, branch)
}

/// `(cwd, gitBranch)` of the session transcript at `path`; `(None, None)` if unreadable.
pub fn session_location(path: &Path) -> (Option<String>, Option<String>) {
    read_tail(path, TAIL_WINDOW).map(|t| latest_location(&t)).unwrap_or_default()
}

#[cfg(test)]
mod tests;
