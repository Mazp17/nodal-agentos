//! Full subagent/session transcript parsing, and the session title Claude Code assigns.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use nodal_domain::diff::{DiffFileStatus, DiffHunk, DiffLine, DiffLineKind, FileDiff};
use nodal_domain::model::claude::{
    AgentStats, SubagentRef, ToolPatch, ToolResultInfo, Transcript, TranscriptItem,
};
use nodal_domain::sessions::transcript::{clip, TEXT_MAX};

use super::last_tool::tool_summary_n;
use super::{lenient, truncate};

/// Above this, the head (prompt) and the tail (recent conversation) are read.
const TRANSCRIPT_MAX_READ: u64 = 16 * 1024 * 1024;
const TRANSCRIPT_HEAD: u64 = 1024 * 1024;
const PROMPT_MAX: usize = 8000;
const TOOL_INPUT_MAX: usize = 2000;
const TOOL_RESULT_MAX: usize = 1500;
const SUMMARY_MAX: usize = 160;

#[derive(Deserialize)]
struct RawTranscriptLine {
    #[serde(default, rename = "type", deserialize_with = "lenient")]
    kind: Option<String>,
    #[serde(default)]
    message: Option<Value>,
    #[serde(default, deserialize_with = "lenient")]
    uuid: Option<String>,
}

/// Just what links a line to the conversation tree (`parentUuid` is null at a compaction
/// boundary, where `logicalParentUuid` says what it continues).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawLink {
    #[serde(default, deserialize_with = "lenient")]
    uuid: Option<String>,
    #[serde(default, rename = "type", deserialize_with = "lenient")]
    kind: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    parent_uuid: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    logical_parent_uuid: Option<String>,
}

/// Structured result of a line's tool call (for `Edit`/`Write`: `structuredPatch`). Read in a
/// second pass, only for lines that carry a patch or an `agentId`: other results (`Read`, the
/// `originalFile` of edits) can be large and are never shown.
#[derive(Deserialize)]
struct RawToolUseResult {
    #[serde(default, rename = "toolUseResult")]
    tool_use_result: Option<Value>,
}

/// The result of parsing the transcript text.
#[derive(Debug, Default)]
pub struct ParsedTranscript {
    pub prompt: Option<String>,
    /// Uuid of the line `prompt` starts at.
    pub prompt_uuid: Option<String>,
    pub items: Vec<TranscriptItem>,
    pub total: usize,
    /// `tool_use` blocks in what was read, including the ones dropped by `limit`.
    pub tool_calls: usize,
    pub final_output: Option<String>,
}

/// The workflow wraps the task: `[Workflow harness — computed task] ... follows:` and the
/// text indented by two spaces. Returns the text without the wrapper.
fn unwrap_harness(text: &str) -> Option<String> {
    if !text.starts_with("[Workflow harness") || !text.contains("computed task]") {
        return None;
    }
    let (_, body) = text.split_once("follows:\n")?;
    Some(
        body.lines()
            .map(|l| l.strip_prefix("  ").unwrap_or(l))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn is_harness_request(text: &str) -> bool {
    text.starts_with("[Workflow harness") && text.contains("user request]")
}

pub(crate) fn tool_result_text(block: &Value) -> String {
    match block.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| match p.get("type").and_then(Value::as_str) {
                Some("text") => p.get("text").and_then(Value::as_str).map(str::to_string),
                Some("image") => Some("[image]".into()),
                Some("tool_reference") => Some(format!(
                    "[tool: {}]",
                    p.get("tool_name").and_then(Value::as_str).unwrap_or("?")
                )),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// A `tool_result` content block as `(tool_use_id, result)`. Session files and stream-json
/// events share this block shape. `structured` is the message's structured tool result, which
/// Claude Code records next to the content, not inside the block: pass it only when the message
/// carries this single `tool_result`, so it can't be attached to the wrong call.
pub(crate) fn tool_result_info<'a>(
    b: &'a Value,
    structured: Option<&Value>,
) -> (Option<&'a str>, ToolResultInfo) {
    let (text, truncated) = clip(&tool_result_text(b), TOOL_RESULT_MAX);
    let is_error = b.get("is_error").and_then(Value::as_bool).unwrap_or(false);
    let patch = if is_error {
        None
    } else {
        structured.and_then(tool_patch).map(Box::new)
    };
    (
        b.get("tool_use_id").and_then(Value::as_str),
        ToolResultInfo {
            text,
            is_error,
            truncated,
            patch,
            agent: structured.and_then(agent_stats).map(Box::new),
        },
    )
}

/// `{status, agentId, agentType, totalDurationMs, totalTokens, totalToolUseCount}` (observed in
/// 2.1.283) of an `Agent`/`Task` result.
fn agent_stats(r: &Value) -> Option<AgentStats> {
    let s = |k: &str| r.get(k).and_then(Value::as_str).map(str::to_string);
    let n = |k: &str| r.get(k).and_then(Value::as_i64);
    Some(AgentStats {
        agent_id: s("agentId")?,
        agent_type: s("agentType"),
        status: s("status"),
        duration_ms: n("totalDurationMs"),
        total_tokens: n("totalTokens"),
        tool_uses: n("totalToolUseCount"),
    })
}

/// The message's structured tool result when it carries exactly one `tool_result` block.
pub(crate) fn single_result<'a>(
    blocks: &[Value],
    structured: Option<&'a Value>,
) -> Option<&'a Value> {
    let n = blocks
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
        .count();
    structured.filter(|_| n == 1)
}

/// Diff lines kept per tool change; the counts still cover everything.
const PATCH_LINES_MAX: usize = 400;
/// Characters kept per diff line.
const PATCH_LINE_MAX: usize = 1000;

/// The file change of an `Edit`/`MultiEdit`/`Write` result: `{filePath, structuredPatch:
/// [{oldStart, oldLines, newStart, newLines, lines: [" ctx", "-old", "+new"]}]}`. A `Write`
/// that creates a file has an empty patch and the whole `content` (verified with 2.1.283).
/// `None` for any other result or when nothing changed.
pub(crate) fn tool_patch(r: &Value) -> Option<ToolPatch> {
    let path = r.get("filePath").and_then(Value::as_str)?.to_string();
    let created = r.get("type").and_then(Value::as_str) == Some("create");
    let raw = r.get("structuredPatch").and_then(Value::as_array)?;
    let n = |h: &Value, k: &str| {
        h.get(k)
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0)
    };
    let mut hunks: Vec<(u32, u32, u32, u32, Vec<String>)> = raw
        .iter()
        .map(|h| {
            let lines = h
                .get("lines")
                .and_then(Value::as_array)
                .into_iter()
                .flatten();
            let lines = lines
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
            (
                n(h, "oldStart"),
                n(h, "oldLines"),
                n(h, "newStart"),
                n(h, "newLines"),
                lines,
            )
        })
        .collect();
    // A created file: every line is an addition. Only the kept lines are copied.
    let mut created_lines = 0u32;
    if hunks.is_empty() && created {
        let content = r.get("content").and_then(Value::as_str).unwrap_or("");
        created_lines = u32::try_from(content.lines().count()).unwrap_or(u32::MAX);
        let lines: Vec<String> = content
            .lines()
            .take(PATCH_LINES_MAX)
            .map(|l| format!("+{l}"))
            .collect();
        if !lines.is_empty() {
            hunks.push((0, 0, 1, created_lines, lines));
        }
    }
    if hunks.is_empty() {
        return None;
    }

    let (mut additions, mut deletions, mut kept) = (0u32, 0u32, 0usize);
    let mut truncated = false;
    let mut out = Vec::new();
    for (old_start, old_lines, new_start, new_lines, lines) in hunks {
        let (mut old_no, mut new_no) = (old_start, new_start);
        let mut body = Vec::new();
        for l in &lines {
            let (kind, text) = match l.chars().next() {
                Some('+') => (DiffLineKind::Add, &l[1..]),
                Some('-') => (DiffLineKind::Del, &l[1..]),
                Some(' ') => (DiffLineKind::Context, &l[1..]),
                // "\ No newline at end of file" and anything unknown.
                _ => continue,
            };
            let (o, nn) = match kind {
                DiffLineKind::Add => (None, Some(new_no)),
                DiffLineKind::Del => (Some(old_no), None),
                DiffLineKind::Context => (Some(old_no), Some(new_no)),
            };
            match kind {
                DiffLineKind::Add => additions += 1,
                DiffLineKind::Del => deletions += 1,
                DiffLineKind::Context => {}
            }
            if o.is_some() {
                old_no += 1;
            }
            if nn.is_some() {
                new_no += 1;
            }
            if kept >= PATCH_LINES_MAX {
                truncated = true;
                continue;
            }
            kept += 1;
            let text = text.strip_suffix('\r').unwrap_or(text);
            body.push(DiffLine {
                kind,
                text: truncate(text, PATCH_LINE_MAX),
                old_no: o,
                new_no: nn,
            });
        }
        if !body.is_empty() {
            let header = format!("@@ -{old_start},{old_lines} +{new_start},{new_lines} @@");
            out.push(DiffHunk {
                header,
                old_start,
                old_lines,
                new_start,
                new_lines,
                lines: body,
            });
        }
    }
    if created_lines as usize > PATCH_LINES_MAX {
        additions = created_lines;
        truncated = true;
    }
    let status = if created {
        DiffFileStatus::Added
    } else {
        DiffFileStatus::Modified
    };
    let file = FileDiff {
        path,
        old_path: None,
        status,
        additions,
        deletions,
        binary: false,
        hunks: out,
    };
    Some(ToolPatch { file, truncated })
}

/// A content block of an assistant message (`text`, `thinking` or `tool_use`) as a
/// transcript item; `None` for empty or unknown blocks. Shared with stream-json events.
pub(crate) fn assistant_item(b: &Value) -> Option<TranscriptItem> {
    match b.get("type").and_then(Value::as_str) {
        Some("text") => {
            let s = b.get("text").and_then(Value::as_str).unwrap_or("").trim();
            if s.is_empty() {
                return None;
            }
            let (text, truncated) = clip(s, TEXT_MAX);
            Some(TranscriptItem::Text { text, truncated })
        }
        Some("thinking") => {
            let s = b
                .get("thinking")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            if s.is_empty() {
                return None;
            }
            let (text, truncated) = clip(s, TEXT_MAX);
            Some(TranscriptItem::Thinking { text, truncated })
        }
        Some("tool_use") => {
            let name = b.get("name").and_then(Value::as_str)?;
            let input = b.get("input");
            let pretty = input
                .filter(|v| v.as_object().is_none_or(|o| !o.is_empty()))
                .and_then(|v| serde_json::to_string_pretty(v).ok());
            Some(TranscriptItem::ToolUse {
                id: b.get("id").and_then(Value::as_str).map(str::to_string),
                name: name.to_string(),
                summary: input.and_then(|i| tool_summary_n(i, SUMMARY_MAX)),
                input: pretty.map(|p| truncate(&p, TOOL_INPUT_MAX)),
                result: None,
            })
        }
        _ => None,
    }
}

/// One line with a tool input's main argument, as transcript tool calls show it.
pub(crate) fn tool_input_summary(input: &Value) -> Option<String> {
    tool_summary_n(input, SUMMARY_MAX)
}

/// Parses the lines of an `agent-<id>.jsonl`: initial prompt, conversation with each
/// `tool_result` attached to its `tool_use`, and final output. Returns at most the last
/// `limit` items. Tolerates broken lines and unknown types.
pub fn parse_transcript(text: &str, limit: usize) -> ParsedTranscript {
    let mut prompts: Vec<String> = Vec::new();
    let mut prompt_uuid: Option<String> = None;
    let mut items: Vec<TranscriptItem> = Vec::new();
    let mut seen_assistant = false;
    let mut last_text: Option<String> = None;
    let mut structured: Option<String> = None;
    let mut tool_calls = 0;

    for line in text.lines() {
        // Cheap filter: attachments (skill lists, snapshots) make up most of the file.
        if line.trim().is_empty() || !(line.contains("\"assistant\"") || line.contains("\"user\""))
        {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<RawTranscriptLine>(line) else {
            continue;
        };
        let content = entry.message.as_ref().and_then(|m| m.get("content"));
        match entry.kind.as_deref() {
            Some("user") => match content {
                Some(Value::String(s)) => {
                    if seen_assistant {
                        let (text, truncated) = clip(s, TEXT_MAX);
                        items.push(TranscriptItem::User { text, truncated, uuid: entry.uuid.clone() });
                    } else {
                        if prompts.is_empty() {
                            prompt_uuid = entry.uuid.clone();
                        }
                        prompts.push(s.clone());
                    }
                }
                Some(Value::Array(blocks)) => {
                    let tool_use_result = (line.contains("\"structuredPatch\"")
                        || line.contains("\"agentId\""))
                        .then(|| serde_json::from_str::<RawToolUseResult>(line).ok())
                        .flatten()
                        .and_then(|r| r.tool_use_result);
                    let structured = single_result(blocks, tool_use_result.as_ref());
                    for b in blocks {
                        match b.get("type").and_then(Value::as_str) {
                            Some("tool_result") => {
                                let (id, info) = tool_result_info(b, structured);
                                // The tool_use is usually very close: search from the end.
                                let slot = items.iter_mut().rev().find_map(|it| match it {
                                    TranscriptItem::ToolUse {
                                        id: Some(tid),
                                        result,
                                        ..
                                    } if Some(tid.as_str()) == id => Some(result),
                                    _ => None,
                                });
                                if let Some(slot) = slot {
                                    *slot = Some(info);
                                }
                            }
                            Some("text") => {
                                let Some(s) = b.get("text").and_then(Value::as_str) else {
                                    continue;
                                };
                                if seen_assistant {
                                    let (text, truncated) = clip(s, TEXT_MAX);
                                    items.push(TranscriptItem::User { text, truncated, uuid: entry.uuid.clone() });
                                } else {
                                    if prompts.is_empty() {
                                        prompt_uuid = entry.uuid.clone();
                                    }
                                    prompts.push(s.to_string());
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            },
            Some("assistant") => {
                seen_assistant = true;
                let Some(Value::Array(blocks)) = content else {
                    continue;
                };
                for b in blocks {
                    let Some(item) = assistant_item(b) else {
                        continue;
                    };
                    match &item {
                        TranscriptItem::Text { text, .. } => last_text = Some(text.clone()),
                        TranscriptItem::ToolUse { name, .. } => {
                            tool_calls += 1;
                            if name == "StructuredOutput" {
                                structured = b
                                    .get("input")
                                    .filter(|v| v.as_object().is_none_or(|o| !o.is_empty()))
                                    .and_then(|v| serde_json::to_string_pretty(v).ok());
                            }
                        }
                        _ => {}
                    }
                    items.push(item);
                }
            }
            _ => {}
        }
    }

    let prompt = prompts
        .iter()
        .find_map(|p| unwrap_harness(p))
        .or_else(|| {
            let rest: Vec<&str> = prompts
                .iter()
                .filter(|p| !is_harness_request(p))
                .map(String::as_str)
                .collect();
            (!rest.is_empty()).then(|| rest.join("\n\n"))
        })
        .map(|p| truncate(p.trim(), PROMPT_MAX));

    let total = items.len();
    if total > limit {
        items.drain(..total - limit);
    }
    ParsedTranscript {
        prompt,
        prompt_uuid,
        items,
        total,
        tool_calls,
        final_output: structured.or(last_text).map(|s| truncate(&s, TEXT_MAX)),
    }
}

/// Characters kept of a session's title.
const SESSION_TITLE_MAX: usize = 120;

/// Claude Code's name for a session, from its `.jsonl`: the last `custom-title` (set with
/// `/rename`) or, without one, the last `ai-title`, which it rewrites as the conversation
/// goes (observed in 2.1.283: `{"type":"ai-title","aiTitle":"…"}`,
/// `{"type":"custom-title","customTitle":"…"}`).
/// Streams the file line by line (it's read after every turn), parsing only title lines.
pub fn session_title(path: &Path) -> Option<String> {
    use std::io::BufRead;
    let mut reader = std::io::BufReader::new(fs::File::open(path).ok()?);
    let (mut custom, mut ai) = (None, None);
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if !line.windows(7).any(|w| w == b"-title\"") {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        let (slot, key) = match v.get("type").and_then(Value::as_str) {
            Some("custom-title") => (&mut custom, "customTitle"),
            Some("ai-title") => (&mut ai, "aiTitle"),
            _ => continue,
        };
        let one_line = v
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if !one_line.is_empty() {
            *slot = Some(truncate(&one_line, SESSION_TITLE_MAX));
        }
    }
    custom.or(ai)
}

/// Reads the whole file, or if it's too large, its head (where the prompt is) and its
/// tail. Returns `(text, partial, bytes)`.
pub(crate) fn read_transcript_text(path: &Path) -> std::io::Result<(String, bool, u64)> {
    let mut file = fs::File::open(path)?;
    let len = file.metadata()?.len();
    if len <= TRANSCRIPT_MAX_READ {
        let mut buf = Vec::with_capacity(len as usize);
        file.read_to_end(&mut buf)?;
        return Ok((String::from_utf8_lossy(&buf).into_owned(), false, len));
    }
    let mut head = Vec::new();
    file.by_ref().take(TRANSCRIPT_HEAD).read_to_end(&mut head)?;
    let head = String::from_utf8_lossy(&head);
    // Only complete lines from the head.
    let head = head.rsplit_once('\n').map_or("", |(h, _)| h);
    let tail_len = TRANSCRIPT_MAX_READ - TRANSCRIPT_HEAD;
    file.seek(SeekFrom::Start(len - tail_len))?;
    let mut tail = Vec::new();
    file.take(tail_len).read_to_end(&mut tail)?;
    let tail = String::from_utf8_lossy(&tail);
    let tail = tail.split_once('\n').map_or("", |(_, r)| r);
    Ok((format!("{head}\n{tail}"), true, len))
}

/// Without the sidechain lines (subagents launched with `Task` inside the session). Claude
/// Code writes compact JSON, so searching for the literal is enough.
fn main_thread_lines(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|l| !l.contains("\"isSidechain\":true"))
        .collect()
}

/// The lines of the active branch, in file order. Rewinding with `--resume-session-at` appends
/// to the same file from an earlier message, leaving the abandoned branch in it: the active
/// one ends at the last line with a `uuid` and is walked back through `parentUuid`. Lines
/// without a `uuid` (titles, snapshots…) are kept. `None` if the walk can't be resolved.
fn active_branch<'a>(lines: &[&'a str]) -> Option<Vec<&'a str>> {
    let links: Vec<Option<RawLink>> = lines
        .iter()
        .map(|l| {
            // Cheap filter, as in `parse_transcript`: only conversation lines have a `uuid`.
            l.contains("\"uuid\"").then(|| serde_json::from_str::<RawLink>(l).ok()).flatten()
        })
        .collect();
    let parents: std::collections::HashMap<&str, Option<&str>> = links
        .iter()
        .flatten()
        .filter_map(|l| {
            let parent = l.parent_uuid.as_deref().or(l.logical_parent_uuid.as_deref());
            Some((l.uuid.as_deref()?, parent))
        })
        .collect();
    let mut cur = links.iter().rev().flatten().find_map(|l| l.uuid.as_deref());
    let mut chain = std::collections::HashSet::new();
    while let Some(id) = cur {
        if !chain.insert(id) {
            return None;
        }
        // A parent that isn't in the file (a truncated one) can't be walked: don't guess.
        cur = *parents.get(id)?;
    }
    Some(
        lines
            .iter()
            .zip(&links)
            .filter(|(_, l)| match l.as_ref().and_then(|l| l.uuid.as_deref()) {
                Some(id) => chain.contains(id),
                None => true,
            })
            .map(|(l, _)| *l)
            .collect(),
    )
}

/// `(name, input)` of the main thread's tool calls after the last user prompt (a user line
/// with text, not a tool result or a meta line), in file order. Empty if unreadable.
pub fn last_turn_tools(path: &Path) -> Vec<(String, Value)> {
    let Ok((text, _, _)) = read_transcript_text(path) else { return vec![] };
    let mut calls = vec![];
    for line in main_thread_lines(&text).into_iter().rev() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let content = v.pointer("/message/content");
        match v.get("type").and_then(Value::as_str) {
            Some("assistant") => {
                let blocks = content.and_then(Value::as_array).into_iter().flatten().rev();
                for b in blocks.filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use")) {
                    let name = b.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
                    calls.push((name, b.get("input").cloned().unwrap_or(Value::Null)));
                }
            }
            Some("user") if v.get("isMeta").and_then(Value::as_bool) != Some(true) => {
                let prompt = match content {
                    Some(Value::String(_)) => true,
                    Some(Value::Array(a)) => a.iter().any(|b| b.get("type").and_then(Value::as_str) == Some("text")),
                    _ => false,
                };
                if prompt {
                    break;
                }
            }
            _ => {}
        }
    }
    calls.reverse();
    calls
}

/// The assistant message right before the user message `user_uuid` on the active branch (what
/// `--resume-session-at` takes to rewind the conversation to just before it); `None` if that
/// message is the first of the session.
pub fn rewind_point(path: &Path, user_uuid: &str) -> Result<Option<String>, String> {
    let (text, partial, _) = read_transcript_text(path).map_err(|e| format!("Couldn't read the transcript: {e}"))?;
    // shortcut: a session over TRANSCRIPT_MAX_READ can't be rewound, upgrade to streaming the file backwards.
    if partial {
        return Err("This conversation is too long to rewind.".into());
    }
    let main = main_thread_lines(&text);
    let branch = active_branch(&main).ok_or("Couldn't follow this conversation's history to rewind it.")?;
    let links: Vec<RawLink> = branch.iter().filter_map(|l| serde_json::from_str(l).ok()).collect();
    let at = links
        .iter()
        .position(|l| l.uuid.as_deref() == Some(user_uuid) && l.kind.as_deref() == Some("user"))
        .ok_or("That message isn't in the conversation anymore.")?;
    Ok(links[..at]
        .iter()
        .rev()
        .find(|l| l.kind.as_deref() == Some("assistant"))
        .and_then(|l| l.uuid.clone()))
}

/// Main transcript of a session (`<sid>.jsonl`): agent, Claude or reviewer runs.
/// `id`/`label`/`model` go as is into the `Transcript`. `Ok(None)` if there's no file.
pub fn read_session_transcript(
    path: &Path,
    id: &str,
    label: Option<String>,
    model: Option<String>,
    limit: usize,
) -> Result<Option<Transcript>, String> {
    let (text, partial, bytes) = match read_transcript_text(path) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Couldn't read the transcript: {e}")),
    };
    let main = main_thread_lines(&text);
    // shortcut: a partial read (head + tail) keeps every branch, as it can't be walked back.
    let lines = if partial { None } else { active_branch(&main) }.unwrap_or(main);
    let parsed = parse_transcript(&lines.join("\n"), limit);
    Ok(Some(Transcript {
        agent_id: id.to_string(),
        label,
        model,
        phase: None,
        prompt: parsed.prompt,
        prompt_uuid: parsed.prompt_uuid,
        omitted: (parsed.total - parsed.items.len()) as u32,
        total_items: parsed.total as u32,
        tool_calls: parsed.tool_calls as u32,
        items: parsed.items,
        final_output: parsed.final_output,
        partial,
        bytes,
    }))
}

fn subagents_dir(session_path: &Path) -> std::path::PathBuf {
    session_path.with_extension("").join("subagents")
}

/// Subagents of `<dir>/<sid>.jsonl`, from `<dir>/<sid>/subagents/agent-<id>.meta.json`, oldest
/// first. The `workflows/` subfolder is ignored.
pub fn list_subagents(session_path: &Path) -> Vec<SubagentRef> {
    let Ok(entries) = fs::read_dir(subagents_dir(session_path)) else {
        return vec![];
    };
    let mut out: Vec<SubagentRef> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let agent_id = name.strip_prefix("agent-")?.strip_suffix(".jsonl")?.to_string();
            if !is_agent_id(&agent_id) {
                return None;
            }
            let meta: Value = fs::read_to_string(e.path().with_extension("meta.json"))
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or(Value::Null);
            let s = |k: &str| meta.get(k).and_then(Value::as_str).map(str::to_string);
            let md = e.metadata().ok()?;
            let started_at = md
                .created()
                .or_else(|_| md.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64);
            Some(SubagentRef {
                agent_id,
                tool_use_id: s("toolUseId"),
                agent_type: s("agentType"),
                description: s("description"),
                model: s("model"),
                stopped_by_user: meta.get("stoppedByUser").and_then(Value::as_bool).unwrap_or(false),
                started_at,
            })
        })
        .collect();
    out.sort_by(|a, b| (a.started_at, &a.agent_id).cmp(&(b.started_at, &b.agent_id)));
    out
}

/// Agent ids are alphanumeric; checked before one goes into a path.
fn is_agent_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// A subagent's own transcript. `Ok(None)` if the id is invalid or there's no file.
pub fn read_subagent_transcript(
    session_path: &Path,
    agent_id: &str,
    limit: usize,
) -> Result<Option<Transcript>, String> {
    if !is_agent_id(agent_id) {
        return Ok(None);
    }
    let file = subagents_dir(session_path).join(format!("agent-{agent_id}.jsonl"));
    let meta = list_subagents(session_path).into_iter().find(|s| s.agent_id == agent_id);
    let (label, model) = meta.map_or((None, None), |m| (m.description, m.model));
    // Lines are all sidechain, so `main_thread_lines`' filter doesn't apply.
    let (text, partial, bytes) = match read_transcript_text(&file) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Couldn't read the transcript: {e}")),
    };
    let parsed = parse_transcript(&text, limit);
    Ok(Some(Transcript {
        agent_id: agent_id.to_string(),
        label,
        model,
        phase: None,
        prompt: parsed.prompt,
        prompt_uuid: None,
        omitted: (parsed.total - parsed.items.len()) as u32,
        total_items: parsed.total as u32,
        tool_calls: parsed.tool_calls as u32,
        items: parsed.items,
        final_output: parsed.final_output,
        partial,
        bytes,
    }))
}

#[cfg(test)]
mod tests;
