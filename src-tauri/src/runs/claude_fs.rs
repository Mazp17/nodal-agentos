//! WARNING: Claude Code's internal, UNDOCUMENTED format (observed in v2.1.281).
//!
//! Everything that depends on how Claude Code prints its output or writes its files lives
//! here and only here, so that when it changes there's a single place to touch:
//! - `claude --bg` output (the `backgrounded · <id>` line),
//! - `claude agents --json --all` output,
//! - layout of `~/.claude/projects/<slug>/<sessionId>/` (journal, meta, transcripts,
//!   final summary `workflows/wf_*.json`, script `workflows/scripts/*-wf_*.js`).
//!
//! Policy: tolerate anything unknown (new fields, unexpected types, lines cut off
//! mid-write) and degrade to `None` instead of failing.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use super::types::{
    AgentInfo, AgentState, DetailSource, PhaseInfo, RunDetail, RunResult, RunSummary, ToolResultInfo, Transcript,
    TranscriptItem,
};

/// Deserializes an optional field without failing if the type isn't the expected one.
fn lenient<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let v = Value::deserialize(d)?;
    Ok(serde_json::from_value(v).ok())
}

// ---------------------------------------------------------------------------
// `claude --bg`
// ---------------------------------------------------------------------------

/// Extracts the short id from a line like `backgrounded · ddb91222` (with or without ANSI colors).
pub fn parse_bg_line(line: &str) -> Option<String> {
    let clean = strip_ansi(line);
    let pos = clean.find("backgrounded")?;
    let rest = &clean[pos + "backgrounded".len()..];
    // First token after `backgrounded` (there may be text after the id).
    let id = rest
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .find(|t| !t.is_empty())?;
    is_plausible_id(id).then(|| id.to_string())
}

/// Fallback if the output lacks the word `backgrounded` (e.g. another format without a TTY):
/// accept output that is just a hex id. Not observed, purely defensive.
pub fn parse_bare_id(output: &str) -> Option<String> {
    let clean = strip_ansi(output);
    let t = clean.trim();
    (t.len() >= 6 && t.len() <= 16 && t.chars().all(|c| c.is_ascii_hexdigit())).then(|| t.to_string())
}

fn is_plausible_id(id: &str) -> bool {
    (4..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                // CSI: parameters up to a final letter.
                for n in chars.by_ref() {
                    if n.is_ascii_alphabetic() || n == '~' {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

// ---------------------------------------------------------------------------
// `claude agents --json --all`
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAgent {
    #[serde(default, deserialize_with = "lenient")]
    id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    session_id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    cwd: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    kind: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    started_at: Option<f64>,
    #[serde(default, deserialize_with = "lenient")]
    name: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pid: Option<u32>,
    #[serde(default, deserialize_with = "lenient")]
    status: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    state: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    waiting_for: Option<String>,
}

/// Sessions with `kind == "background"`, most recent first. Entries without `id` or
/// `sessionId` are dropped (interactive ones, for example, have no `id`).
pub fn parse_agents_json(text: &str) -> Result<Vec<RunSummary>, String> {
    let values: Vec<Value> = serde_json::from_str(text.trim())
        .map_err(|e| format!("`claude agents --json` didn't return the expected JSON list: {e}"))?;
    let mut runs: Vec<RunSummary> = values
        .into_iter()
        .filter_map(|v| serde_json::from_value::<RawAgent>(v).ok())
        .filter(|a| a.kind.as_deref() == Some("background"))
        .filter_map(|a| {
            Some(RunSummary {
                id: a.id?,
                session_id: a.session_id?,
                cwd: a.cwd,
                name: a.name,
                started_at: a.started_at.map(|n| n as i64),
                pid: a.pid,
                status: a.status,
                state: a.state,
                waiting_for: a.waiting_for,
            })
        })
        .collect();
    runs.sort_by_key(|r| std::cmp::Reverse(r.started_at));
    Ok(runs)
}

// ---------------------------------------------------------------------------
// Session location on disk
// ---------------------------------------------------------------------------

/// `$CLAUDE_CONFIG_DIR` or `~/.claude`.
pub fn claude_config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".claude"))
}

/// Project directory slug: every non-alphanumeric character becomes `-`.
/// Verified against `~/.claude/projects`: `/Users/x/Code/nodal-sandbox` →
/// `-Users-x-Code-nodal-sandbox`, and `/.claude/worktrees` → `--claude-worktrees`.
pub fn project_slug(cwd: &str) -> String {
    cwd.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

/// A sessionId comes from the frontend and ends up in a path: only `[A-Za-z0-9-]`.
pub fn is_valid_session_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// `<projects>/<slug>/<sessionId>`. If the slug doesn't match (long paths or changed
/// rules), looks for the session in any project.
pub fn find_session_dir(projects: &Path, cwd: &str, session_id: &str) -> Option<PathBuf> {
    let direct = projects.join(project_slug(cwd)).join(session_id);
    if direct.is_dir() {
        return Some(direct);
    }
    fs::read_dir(projects)
        .ok()?
        .flatten()
        .map(|e| e.path().join(session_id))
        .find(|p| p.is_dir())
}

// ---------------------------------------------------------------------------
// Workflow detail
// ---------------------------------------------------------------------------

/// Detail of the session's most recent workflow, or `None` if the session launched none.
pub fn read_run_detail(session_dir: &Path) -> Option<RunDetail> {
    let ids = workflow_ids(session_dir);
    let workflow_count = ids.len() as u32;
    let wf_id = ids
        .into_iter()
        .max_by_key(|id| (workflow_mtime(session_dir, id), id.clone()))?;

    let final_path = session_dir.join("workflows").join(format!("{wf_id}.json"));
    let from_final = fs::read_to_string(&final_path)
        .ok()
        .and_then(|text| parse_final(&text, &wf_id));
    let mut detail = match from_final {
        Some(d) => d,
        // No summary (or half-written): rebuilt from the journal.
        None => read_live(session_dir, &wf_id),
    };
    detail.workflow_count = workflow_count;
    Some(detail)
}

fn wf_live_dir(session_dir: &Path, wf_id: &str) -> PathBuf {
    session_dir.join("subagents").join("workflows").join(wf_id)
}

/// `wf_*` ids present in `subagents/workflows/` (dirs) and `workflows/` (summaries).
fn workflow_ids(session_dir: &Path) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(session_dir.join("subagents").join("workflows")) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with("wf_") && e.path().is_dir() {
                ids.push(name);
            }
        }
    }
    if let Ok(entries) = fs::read_dir(session_dir.join("workflows")) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if let Some(id) = name.strip_suffix(".json").filter(|n| n.starts_with("wf_")) {
                ids.push(id.to_string());
            }
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

fn workflow_mtime(session_dir: &Path, wf_id: &str) -> SystemTime {
    let live = wf_live_dir(session_dir, wf_id);
    [
        live.join("journal.jsonl"),
        live,
        session_dir.join("workflows").join(format!("{wf_id}.json")),
    ]
    .iter()
    .filter_map(|p| fs::metadata(p).and_then(|m| m.modified()).ok())
    .max()
    .unwrap_or(SystemTime::UNIX_EPOCH)
}

fn map_state(raw: Option<&str>) -> AgentState {
    match raw {
        Some("done" | "completed" | "success") => AgentState::Done,
        Some("running" | "working" | "in_progress" | "started") => AgentState::Running,
        Some("queued" | "pending" | "waiting") => AgentState::Queued,
        Some("failed" | "error" | "errored" | "cancelled" | "killed") => AgentState::Failed,
        _ => AgentState::Unknown,
    }
}

fn result_status(result: Option<&Value>) -> Option<String> {
    let s = result?.get("status")?.as_str()?;
    matches!(s, "green" | "yellow" | "red").then(|| s.to_string())
}

fn phase_index(phases: &[PhaseInfo], title: Option<&str>) -> Option<u32> {
    let title = title?;
    phases.iter().position(|p| p.title == title).map(|i| i as u32 + 1)
}

// --- Final summary: workflows/wf_<id>.json --------------------------------

#[derive(Deserialize)]
struct RawPhase {
    #[serde(default, deserialize_with = "lenient")]
    title: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    detail: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawFinal {
    #[serde(default, deserialize_with = "lenient")]
    run_id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    workflow_name: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    status: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    phases: Option<Vec<RawPhase>>,
    #[serde(default, deserialize_with = "lenient")]
    workflow_progress: Option<Vec<Value>>,
    #[serde(default, deserialize_with = "lenient")]
    agent_count: Option<u32>,
    #[serde(default, deserialize_with = "lenient")]
    total_tokens: Option<u64>,
    #[serde(default, deserialize_with = "lenient")]
    total_tool_calls: Option<u64>,
    #[serde(default, deserialize_with = "lenient")]
    duration_ms: Option<u64>,
    #[serde(default)]
    result: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawProgressItem {
    #[serde(default, rename = "type", deserialize_with = "lenient")]
    kind: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    title: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    label: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    phase_title: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    agent_id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    model: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    state: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    tokens: Option<u64>,
    #[serde(default, deserialize_with = "lenient")]
    tool_calls: Option<u64>,
    #[serde(default, deserialize_with = "lenient")]
    duration_ms: Option<u64>,
    #[serde(default, deserialize_with = "lenient")]
    last_tool_name: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    last_tool_summary: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    result_preview: Option<String>,
}

/// `None` if the file isn't valid JSON (e.g. half-written).
pub fn parse_final(text: &str, wf_id: &str) -> Option<RunDetail> {
    let raw: RawFinal = serde_json::from_str(text).ok()?;
    let progress: Vec<RawProgressItem> = raw
        .workflow_progress
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect();

    let mut phases: Vec<PhaseInfo> = raw
        .phases
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| Some(PhaseInfo { title: p.title?, detail: p.detail }))
        .collect();
    if phases.is_empty() {
        phases = progress
            .iter()
            .filter(|p| p.kind.as_deref() == Some("workflow_phase"))
            .filter_map(|p| Some(PhaseInfo { title: p.title.clone()?, detail: None }))
            .collect();
    }

    let agents: Vec<AgentInfo> = progress
        .into_iter()
        .filter(|p| p.kind.as_deref() == Some("workflow_agent"))
        .map(|p| AgentInfo {
            state: map_state(p.state.as_deref()),
            label: p.label.or_else(|| p.agent_id.clone()).unwrap_or_else(|| "(unnamed)".into()),
            agent_id: p.agent_id,
            phase: p.phase_title,
            model: p.model,
            tokens: p.tokens,
            tool_calls: p.tool_calls,
            duration_ms: p.duration_ms,
            last_tool_name: p.last_tool_name,
            last_tool_summary: p.last_tool_summary,
            result_preview: p.result_preview,
        })
        .collect();

    let current_phase = agents.iter().rev().find_map(|a| a.phase.clone());
    Some(RunDetail {
        workflow_id: raw.run_id.unwrap_or_else(|| wf_id.to_string()),
        workflow_name: raw.workflow_name,
        source: DetailSource::Final,
        status: raw.status,
        current_phase_index: phase_index(&phases, current_phase.as_deref()),
        current_phase,
        phases,
        agent_count: raw.agent_count.unwrap_or(agents.len() as u32),
        agents,
        total_tokens: raw.total_tokens,
        total_tool_calls: raw.total_tool_calls,
        duration_ms: raw.duration_ms,
        result_status: result_status(raw.result.as_ref()),
        result: parse_result(raw.result.as_ref()),
        workflow_count: 1,
    })
}

// --- Live: journal.jsonl + meta + transcripts ---------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawJournalLine {
    #[serde(default, rename = "type", deserialize_with = "lenient")]
    kind: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    key: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    agent_id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    label: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    phase: Option<String>,
}

#[derive(Deserialize)]
struct RawMeta {
    #[serde(default, deserialize_with = "lenient")]
    model: Option<String>,
}

/// Journal agents in start order: `started` without `result` = running.
/// Ignores unknown types and unreadable lines (the last one may be half-written).
pub fn parse_journal(text: &str) -> Vec<AgentInfo> {
    let mut agents: Vec<AgentInfo> = Vec::new();
    // Identity of each agent: agentId, or the `key` if missing.
    let mut ids: Vec<String> = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(entry) = serde_json::from_str::<RawJournalLine>(line) else { continue };
        let Some(ident) = entry.agent_id.clone().or(entry.key.clone()) else { continue };
        match entry.kind.as_deref() {
            Some("started") => {
                let info = AgentInfo {
                    agent_id: entry.agent_id,
                    label: entry.label.unwrap_or_else(|| ident.clone()),
                    phase: entry.phase,
                    model: None,
                    state: AgentState::Running,
                    tokens: None,
                    tool_calls: None,
                    duration_ms: None,
                    last_tool_name: None,
                    last_tool_summary: None,
                    result_preview: None,
                };
                // A retry may reuse the identity: the latest start wins.
                if let Some(i) = ids.iter().position(|x| *x == ident) {
                    agents[i] = info;
                } else {
                    ids.push(ident);
                    agents.push(info);
                }
            }
            Some("result") => {
                if let Some(i) = ids.iter().position(|x| *x == ident) {
                    agents[i].state = AgentState::Done;
                }
            }
            _ => {}
        }
    }
    agents
}

fn read_live(session_dir: &Path, wf_id: &str) -> RunDetail {
    let dir = wf_live_dir(session_dir, wf_id);
    // Lossy: the last line may be cut off in the middle of a multibyte character.
    let journal = fs::read(dir.join("journal.jsonl"))
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    let mut agents = parse_journal(&journal);

    for a in agents.iter_mut() {
        let Some(id) = a.agent_id.clone() else { continue };
        a.model = fs::read_to_string(dir.join(format!("agent-{id}.meta.json")))
            .ok()
            .and_then(|t| serde_json::from_str::<RawMeta>(&t).ok())
            .and_then(|m| m.model);
        if a.state == AgentState::Running {
            if let Some((name, summary)) = last_tool_from_transcript(&dir.join(format!("agent-{id}.jsonl"))) {
                a.last_tool_name = Some(name);
                a.last_tool_summary = summary;
            }
        }
    }

    let script = find_script(session_dir, wf_id);
    let mut phases = script
        .as_ref()
        .and_then(|p| fs::read_to_string(p).ok())
        .map(|src| parse_script_phases(&src))
        .unwrap_or_default();
    // No readable script: the phases seen in the journal, in order.
    for a in &agents {
        if let Some(ph) = &a.phase {
            if !phases.iter().any(|p| &p.title == ph) {
                phases.push(PhaseInfo { title: ph.clone(), detail: None });
            }
        }
    }
    let workflow_name = script.as_ref().and_then(|p| {
        let file = p.file_name()?.to_string_lossy().into_owned();
        file.strip_suffix(&format!("-{wf_id}.js")).map(str::to_string)
    });

    let current_phase = agents.iter().rev().find_map(|a| a.phase.clone());
    RunDetail {
        workflow_id: wf_id.to_string(),
        workflow_name,
        source: DetailSource::Live,
        status: None,
        current_phase_index: phase_index(&phases, current_phase.as_deref()),
        current_phase,
        phases,
        agent_count: agents.len() as u32,
        agents,
        total_tokens: None,
        total_tool_calls: None,
        duration_ms: None,
        result_status: None,
        result: None,
        workflow_count: 1,
    }
}

/// `workflows/scripts/<name>-<wf_id>.js`.
fn find_script(session_dir: &Path, wf_id: &str) -> Option<PathBuf> {
    let suffix = format!("-{wf_id}.js");
    fs::read_dir(session_dir.join("workflows").join("scripts"))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().ends_with(&suffix)))
}

/// Phases declared in `meta.phases` of the workflow's JS script. Best effort: it's JS, not
/// JSON; looks for `title:` / `detail:` with string literals inside the array.
pub fn parse_script_phases(src: &str) -> Vec<PhaseInfo> {
    let Some(start) = src.find("phases:") else { return Vec::new() };
    let after = &src[start + "phases:".len()..];
    let Some(open) = after.find('[') else { return Vec::new() };
    if !after[..open].trim().is_empty() {
        return Vec::new();
    }
    let body = &after[open + 1..];
    let Some(end) = find_closing(body, '[', ']') else { return Vec::new() };
    split_top_level_objects(&body[..end])
        .into_iter()
        .filter_map(|obj| {
            Some(PhaseInfo {
                title: js_string_prop(obj, "title")?,
                detail: js_string_prop(obj, "detail"),
            })
        })
        .collect()
}

/// Index of the balancing closer, skipping string literals.
pub(crate) fn find_closing(s: &str, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' | '`' => quote = Some(c),
            c if c == open => depth += 1,
            c if c == close => {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

/// Top-level `{ ... }` chunks, without the braces.
fn split_top_level_objects(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(open) = rest.find('{') {
        let inner = &rest[open + 1..];
        let Some(end) = find_closing(inner, '{', '}') else { break };
        out.push(&inner[..end]);
        rest = &inner[end + 1..];
    }
    out
}

/// Value of `key: '...'` (single quotes, double quotes or backticks).
pub(crate) fn js_string_prop(obj: &str, key: &str) -> Option<String> {
    let mut search = obj;
    loop {
        let pos = search.find(key)?;
        let before_ok = search[..pos]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'));
        let rest = search[pos + key.len()..].trim_start();
        if before_ok {
            if let Some(rest) = rest.strip_prefix(':') {
                let rest = rest.trim_start();
                let Some(q) = rest.chars().next().filter(|c| matches!(c, '\'' | '"' | '`')) else {
                    search = &search[pos + key.len()..];
                    continue;
                };
                let mut out = String::new();
                let mut escaped = false;
                for c in rest[1..].chars() {
                    if escaped {
                        out.push(match c {
                            'n' => '\n',
                            't' => '\t',
                            other => other,
                        });
                        escaped = false;
                    } else if c == '\\' {
                        escaped = true;
                    } else if c == q {
                        return Some(out);
                    } else {
                        out.push(c);
                    }
                }
                return None;
            }
        }
        search = &search[pos + key.len()..];
    }
}

// --- Agent transcript: last tool --------------------------------------------

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
        let text = if start > 0 { text.split_once('\n').map_or("", |(_, r)| r) } else { &text };
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

fn tool_summary(input: &Value) -> Option<String> {
    tool_summary_n(input, 80)
}

fn tool_summary_n(input: &Value, max: usize) -> Option<String> {
    const KEYS: [&str; 9] =
        ["command", "file_path", "pattern", "path", "url", "query", "skill", "description", "prompt"];
    let s = KEYS.iter().find_map(|k| input.get(*k)?.as_str())?;
    let one_line = s.lines().next().unwrap_or("").trim();
    Some(truncate(one_line, max))
}

fn truncate(s: &str, max: usize) -> String {
    clip(s, max).0
}

/// Clips to `max` characters (with `…`) and reports whether it clipped.
fn clip(s: &str, max: usize) -> (String, bool) {
    match s.char_indices().nth(max) {
        None => (s.to_string(), false),
        Some((cut, _)) => {
            let mut out = s[..cut].to_string();
            out.push('…');
            (out, true)
        }
    }
}

// --- Workflow result ----------------------------------------------------------

const RESULT_RAW_MAX: usize = 8000;
const RESULT_LIST_MAX: usize = 100;
const RESULT_ITEM_MAX: usize = 2000;

fn str_field(obj: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key)?.as_str().map(str::trim).filter(|s| !s.is_empty()).map(|s| truncate(s, RESULT_ITEM_MAX))
}

/// List of strings; ignores elements that aren't. `None` if the field isn't an array.
fn str_list(obj: &serde_json::Map<String, Value>, key: &str) -> Option<Vec<String>> {
    let arr = obj.get(key)?.as_array()?;
    Some(
        arr.iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .take(RESULT_LIST_MAX)
            .map(|s| truncate(s, RESULT_ITEM_MAX))
            .collect(),
    )
}

/// Known fields of the `result` (shape of `linear-issue`) plus the clipped raw JSON.
/// `result` is free-form: anything without the expected shape stays `None`.
pub fn parse_result(result: Option<&Value>) -> Option<RunResult> {
    let v = result.filter(|v| !v.is_null())?;
    let raw = match v {
        Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    };
    let mut out = RunResult { raw: Some(truncate(&raw, RESULT_RAW_MAX)), ..RunResult::default() };
    if let Some(obj) = v.as_object() {
        out.issue = str_field(obj, "issue");
        // Web URLs only: the frontend opens it with the system opener.
        // Not clipped: a cut URL would open a broken link.
        out.pr = obj
            .get("pr")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|u| (u.starts_with("https://") || u.starts_with("http://")) && u.len() <= RESULT_ITEM_MAX)
            .map(String::from);
        out.branch = str_field(obj, "branch");
        out.workdir = str_field(obj, "workdir");
        out.where_ = str_field(obj, "where");
        out.unmet_acceptance = str_list(obj, "unmetAcceptance");
        out.nits = str_list(obj, "nits");
    }
    Some(out)
}

// --- Full subagent transcript -------------------------------------------------

/// Above this, the head (prompt) and the tail (recent conversation) are read.
const TRANSCRIPT_MAX_READ: u64 = 16 * 1024 * 1024;
const TRANSCRIPT_HEAD: u64 = 1024 * 1024;
const PROMPT_MAX: usize = 8000;
const TEXT_MAX: usize = 4000;
const TOOL_INPUT_MAX: usize = 2000;
const TOOL_RESULT_MAX: usize = 1500;
const SUMMARY_MAX: usize = 160;
pub const TRANSCRIPT_DEFAULT_LIMIT: u32 = 200;
pub const TRANSCRIPT_MAX_LIMIT: u32 = 2000;

/// Ids that end up in a path (`wf_...`, agent id): only `[A-Za-z0-9_-]`.
pub fn is_valid_path_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && !id.starts_with('-')
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

#[derive(Deserialize)]
struct RawTranscriptLine {
    #[serde(default, rename = "type", deserialize_with = "lenient")]
    kind: Option<String>,
    #[serde(default)]
    message: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAgentMeta {
    #[serde(default, deserialize_with = "lenient")]
    model: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    description: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    workflow_phase: Option<String>,
}

/// The result of parsing the transcript text.
#[derive(Debug, Default)]
pub struct ParsedTranscript {
    pub prompt: Option<String>,
    pub items: Vec<TranscriptItem>,
    pub total: usize,
    pub final_output: Option<String>,
}

/// The workflow wraps the task: `[Workflow harness — computed task] ... follows:` and the
/// text indented by two spaces. Returns the text without the wrapper.
fn unwrap_harness(text: &str) -> Option<String> {
    if !text.starts_with("[Workflow harness") || !text.contains("computed task]") {
        return None;
    }
    let (_, body) = text.split_once("follows:\n")?;
    Some(body.lines().map(|l| l.strip_prefix("  ").unwrap_or(l)).collect::<Vec<_>>().join("\n"))
}

fn is_harness_request(text: &str) -> bool {
    text.starts_with("[Workflow harness") && text.contains("user request]")
}

fn tool_result_text(block: &Value) -> String {
    match block.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| match p.get("type").and_then(Value::as_str) {
                Some("text") => p.get("text").and_then(Value::as_str).map(str::to_string),
                Some("image") => Some("[image]".into()),
                Some("tool_reference") => {
                    Some(format!("[tool: {}]", p.get("tool_name").and_then(Value::as_str).unwrap_or("?")))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Parses the lines of an `agent-<id>.jsonl`: initial prompt, conversation with each
/// `tool_result` attached to its `tool_use`, and final output. Returns at most the last
/// `limit` items. Tolerates broken lines and unknown types.
pub fn parse_transcript(text: &str, limit: usize) -> ParsedTranscript {
    let mut prompts: Vec<String> = Vec::new();
    let mut items: Vec<TranscriptItem> = Vec::new();
    let mut seen_assistant = false;
    let mut last_text: Option<String> = None;
    let mut structured: Option<String> = None;

    for line in text.lines() {
        // Cheap filter: attachments (skill lists, snapshots) make up most of the file.
        if line.trim().is_empty() || !(line.contains("\"assistant\"") || line.contains("\"user\"")) {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<RawTranscriptLine>(line) else { continue };
        let content = entry.message.as_ref().and_then(|m| m.get("content"));
        match entry.kind.as_deref() {
            Some("user") => match content {
                Some(Value::String(s)) => {
                    if seen_assistant {
                        let (text, truncated) = clip(s, TEXT_MAX);
                        items.push(TranscriptItem::User { text, truncated });
                    } else {
                        prompts.push(s.clone());
                    }
                }
                Some(Value::Array(blocks)) => {
                    for b in blocks {
                        match b.get("type").and_then(Value::as_str) {
                            Some("tool_result") => {
                                let id = b.get("tool_use_id").and_then(Value::as_str);
                                let (text, truncated) = clip(&tool_result_text(b), TOOL_RESULT_MAX);
                                let info = ToolResultInfo {
                                    text,
                                    is_error: b.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                                    truncated,
                                };
                                // The tool_use is usually very close: search from the end.
                                let slot = items.iter_mut().rev().find_map(|it| match it {
                                    TranscriptItem::ToolUse { id: Some(tid), result, .. } if Some(tid.as_str()) == id => {
                                        Some(result)
                                    }
                                    _ => None,
                                });
                                if let Some(slot) = slot {
                                    *slot = Some(info);
                                }
                            }
                            Some("text") => {
                                let Some(s) = b.get("text").and_then(Value::as_str) else { continue };
                                if seen_assistant {
                                    let (text, truncated) = clip(s, TEXT_MAX);
                                    items.push(TranscriptItem::User { text, truncated });
                                } else {
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
                let Some(Value::Array(blocks)) = content else { continue };
                for b in blocks {
                    match b.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            let s = b.get("text").and_then(Value::as_str).unwrap_or("").trim();
                            if s.is_empty() {
                                continue;
                            }
                            let (text, truncated) = clip(s, TEXT_MAX);
                            last_text = Some(text.clone());
                            items.push(TranscriptItem::Text { text, truncated });
                        }
                        Some("thinking") => {
                            let s = b.get("thinking").and_then(Value::as_str).unwrap_or("").trim();
                            if !s.is_empty() {
                                let (text, truncated) = clip(s, TEXT_MAX);
                                items.push(TranscriptItem::Thinking { text, truncated });
                            }
                        }
                        Some("tool_use") => {
                            let Some(name) = b.get("name").and_then(Value::as_str) else { continue };
                            let input = b.get("input");
                            let pretty = input
                                .filter(|v| v.as_object().is_none_or(|o| !o.is_empty()))
                                .and_then(|v| serde_json::to_string_pretty(v).ok());
                            if name == "StructuredOutput" {
                                structured = pretty.clone();
                            }
                            items.push(TranscriptItem::ToolUse {
                                id: b.get("id").and_then(Value::as_str).map(str::to_string),
                                name: name.to_string(),
                                summary: input.and_then(|i| tool_summary_n(i, SUMMARY_MAX)),
                                input: pretty.map(|p| truncate(&p, TOOL_INPUT_MAX)),
                                result: None,
                            });
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    let prompt = prompts
        .iter()
        .find_map(|p| unwrap_harness(p))
        .or_else(|| {
            let rest: Vec<&str> = prompts.iter().filter(|p| !is_harness_request(p)).map(String::as_str).collect();
            (!rest.is_empty()).then(|| rest.join("\n\n"))
        })
        .map(|p| truncate(p.trim(), PROMPT_MAX));

    let total = items.len();
    if total > limit {
        items.drain(..total - limit);
    }
    ParsedTranscript {
        prompt,
        items,
        total,
        final_output: structured.or(last_text).map(|s| truncate(&s, TEXT_MAX)),
    }
}

/// Reads the whole file, or if it's too large, its head (where the prompt is) and its
/// tail. Returns `(text, partial, bytes)`.
fn read_transcript_text(path: &Path) -> std::io::Result<(String, bool, u64)> {
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

/// Transcript of agent `agent_id` of workflow `wf_id`. `Ok(None)` if there's no file.
/// The ids must already be validated with `is_valid_path_id`.
pub fn read_agent_transcript(
    session_dir: &Path,
    wf_id: &str,
    agent_id: &str,
    limit: usize,
) -> Result<Option<Transcript>, String> {
    if !is_valid_path_id(wf_id) || !is_valid_path_id(agent_id) {
        return Err("Invalid workflow or agent id.".into());
    }
    let dir = wf_live_dir(session_dir, wf_id);
    let path = dir.join(format!("agent-{agent_id}.jsonl"));
    let (text, partial, bytes) = match read_transcript_text(&path) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Couldn't read the transcript: {e}")),
    };
    let meta = fs::read_to_string(dir.join(format!("agent-{agent_id}.meta.json")))
        .ok()
        .and_then(|t| serde_json::from_str::<RawAgentMeta>(&t).ok());
    let parsed = parse_transcript(&text, limit);
    Ok(Some(Transcript {
        agent_id: agent_id.to_string(),
        label: meta.as_ref().and_then(|m| m.description.clone()),
        model: meta.as_ref().and_then(|m| m.model.clone()),
        phase: meta.and_then(|m| m.workflow_phase),
        prompt: parsed.prompt,
        omitted: (parsed.total - parsed.items.len()) as u32,
        total_items: parsed.total as u32,
        items: parsed.items,
        final_output: parsed.final_output,
        partial,
        bytes,
    }))
}

/// Without the sidechain lines (subagents launched with `Task` inside the session). Claude
/// Code writes compact JSON, so searching for the literal is enough.
fn main_thread_lines(text: &str) -> String {
    text.lines().filter(|l| !l.contains("\"isSidechain\":true")).collect::<Vec<_>>().join("\n")
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
    let parsed = parse_transcript(&main_thread_lines(&text), limit);
    Ok(Some(Transcript {
        agent_id: id.to_string(),
        label,
        model,
        phase: None,
        prompt: parsed.prompt,
        omitted: (parsed.total - parsed.items.len()) as u32,
        total_items: parsed.total as u32,
        items: parsed.items,
        final_output: parsed.final_output,
        partial,
        bytes,
    }))
}

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

// ---------------------------------------------------------------------------
// Why a background session never got to start its workflow
// ---------------------------------------------------------------------------

/// Text Claude Code (2.1.281) uses to reject the `Workflow` tool when the workflow is new
/// or changed and nobody approved it in `/workflows`. In a `--bg` session there's nobody to
/// ask: it stays as a `tool_result` with `is_error` and the session ends without a workflow.
pub const WORKFLOW_REVIEW_TEXT: &str = "Review dynamic workflow before running";
/// How much of the transcript's head is read: the `Workflow` call comes early on.
const BLOCKER_SCAN_BYTES: u64 = 4 * 1024 * 1024;

/// `<projects>/<slug(cwd)>/<sid>.jsonl`, or the first one found in another project.
pub fn find_session_jsonl(projects: &Path, cwd: &str, session_id: &str) -> Option<PathBuf> {
    let file = format!("{session_id}.jsonl");
    let direct = projects.join(project_slug(cwd)).join(&file);
    if direct.is_file() {
        return Some(direct);
    }
    fs::read_dir(projects).ok()?.flatten().map(|e| e.path().join(&file)).find(|p| p.is_file())
}

/// Cap on the returned last message (the final JSON block goes at the end).
const LAST_TEXT_MAX: usize = 64 * 1024;

/// Text of the last assistant message in the lines of a session transcript: the `text`
/// blocks of the last turn with text, joined. Ignores sidechains (subagents).
pub fn last_assistant_text_in(text: &str) -> Option<String> {
    let mut last: Option<String> = None;
    for line in text.lines() {
        if !line.contains("\"assistant\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if v.get("type").and_then(Value::as_str) != Some("assistant")
            || v.get("isSidechain").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        let Some(Value::Array(blocks)) = v.get("message").and_then(|m| m.get("content")) else { continue };
        let joined: Vec<&str> = blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if !joined.is_empty() {
            last = Some(joined.join("\n\n"));
        }
    }
    last.map(|s| {
        if s.len() <= LAST_TEXT_MAX {
            return s;
        }
        // Keep the end, which is where the JSON block goes.
        let mut start = s.len() - LAST_TEXT_MAX;
        while !s.is_char_boundary(start) {
            start += 1;
        }
        s[start..].to_string()
    })
}

/// Last assistant message of the main transcript (`<sid>.jsonl`). Reads at most the
/// file's tail (see `read_transcript_text`).
pub fn read_last_assistant_text(path: &Path) -> Option<String> {
    let (text, _, _) = read_transcript_text(path).ok()?;
    last_assistant_text_in(&text)
}

/// Searches the lines of a session transcript for the `Workflow` call rejected for lack
/// of approval. `Some(name)` if found (the name may be missing).
pub fn find_workflow_review_denial(text: &str) -> Option<Option<String>> {
    // tool_use id → name of the requested workflow.
    let mut calls: Vec<(String, Option<String>)> = Vec::new();
    for line in text.lines() {
        let has_call = line.contains("\"Workflow\"");
        let has_denial = line.contains(WORKFLOW_REVIEW_TEXT);
        if !has_call && !has_denial {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let Some(blocks) = v.pointer("/message/content").and_then(Value::as_array) else { continue };
        for b in blocks {
            match b.get("type").and_then(Value::as_str) {
                Some("tool_use") if b.get("name").and_then(Value::as_str) == Some("Workflow") => {
                    if let Some(id) = b.get("id").and_then(Value::as_str) {
                        let name = b.pointer("/input/name").and_then(Value::as_str).map(str::to_string);
                        calls.push((id.to_string(), name));
                    }
                }
                Some("tool_result") if tool_result_text(b).contains(WORKFLOW_REVIEW_TEXT) => {
                    let id = b.get("tool_use_id").and_then(Value::as_str);
                    let name = calls.iter().find(|(c, _)| Some(c.as_str()) == id).and_then(|(_, n)| n.clone());
                    return Some(name);
                }
                _ => {}
            }
        }
    }
    None
}

/// Reads the head of the transcript and looks for the rejection (a cut-off last line
/// doesn't parse as JSON and is ignored).
pub fn read_workflow_review_denial(path: &Path) -> Option<Option<String>> {
    let file = fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    file.take(BLOCKER_SCAN_BYTES).read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    find_workflow_review_denial(&text)
}

// ---------------------------------------------------------------------------
// Tests: fixtures trimmed from real runs in `src/runs/fixtures/`.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SANDBOX: &str = "/Users/me/Code/nodal-sandbox";
    const DONE_SESSION: &str = "ddb91222-57b2-4ae4-a0bb-d1c5993dc1c8";
    const CUT_SESSION: &str = "91a57808-74f6-40fd-9fbf-184b7358bbe4";

    fn fixtures_projects() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/runs/fixtures/projects")
    }

    #[test]
    fn workflow_review_denial_from_real_session() {
        // Real (trimmed) lines from a claude 2.1.281 --bg session with an unapproved plan-task.
        let text = concat!(
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_01D3","name":"Workflow","input":{"name":"plan-task","args":{"finish":"branch"}}}]}}"#,
            "\n",
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"Review dynamic workflow before running","is_error":true,"tool_use_id":"toolu_01D3"}]},"toolUseResult":"Error: Review dynamic workflow before running"}"#,
            "\n",
        );
        assert_eq!(find_workflow_review_denial(text), Some(Some("plan-task".into())));
        // Without the call (or with another format): still detected, without a name.
        let only = text.lines().nth(1).unwrap();
        assert_eq!(find_workflow_review_denial(only), Some(None));
        // An assistant text quoting the message doesn't count.
        let quoted = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Review dynamic workflow before running"}]}}"#;
        assert_eq!(find_workflow_review_denial(quoted), None);
        assert_eq!(find_workflow_review_denial("{broken\n"), None);
    }

    #[test]
    fn bg_line_real_format() {
        assert_eq!(parse_bg_line("backgrounded · ddb91222").as_deref(), Some("ddb91222"));
        assert_eq!(parse_bg_line("\u{1b}[2mbackgrounded\u{1b}[0m · \u{1b}[1mddb91222\u{1b}[0m").as_deref(), Some("ddb91222"));
        assert_eq!(parse_bg_line("  backgrounded · ab12cd34  \r").as_deref(), Some("ab12cd34"));
        assert_eq!(parse_bg_line("backgrounded · ab12cd34 · run `claude agents` to view").as_deref(), Some("ab12cd34"));
        assert_eq!(parse_bg_line("backgrounded"), None);
        assert_eq!(parse_bg_line("Error: not logged in"), None);
        assert_eq!(parse_bare_id("ddb91222\n").as_deref(), Some("ddb91222"));
        assert_eq!(parse_bare_id("hello world"), None);
    }

    #[test]
    fn agents_json_filters_background_and_tolerates_missing_fields() {
        let text = r#"[
          {"id":"0eef7f11","cwd":"/a","kind":"background","startedAt":1787690543980,
           "sessionId":"0eef7f11-d932-4001-97f8-df01555801d7","name":"old","state":"done"},
          {"pid":59575,"cwd":"/b","kind":"interactive","startedAt":1790087661575,
           "sessionId":"a5ff54f0-6f8e-49b4-9213-0c859d3432de","name":"interactive","status":"idle"},
          {"pid":123,"id":"ddb91222","cwd":"/c","kind":"background","startedAt":1790192548000,
           "sessionId":"ddb91222-57b2-4ae4-a0bb-d1c5993dc1c8","name":"new","status":"busy",
           "state":"working","newField":{"x":1}},
          {"id":"nosession","kind":"background"},
          {"id":"odd","sessionId":"s","kind":"background","pid":"not-a-number","startedAt":"yesterday"}
        ]"#;
        let runs = parse_agents_json(text).unwrap();
        let ids: Vec<&str> = runs.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["ddb91222", "0eef7f11", "odd"]);
        assert_eq!(runs[0].pid, Some(123));
        assert_eq!(runs[0].status.as_deref(), Some("busy"));
        assert_eq!(runs[0].state.as_deref(), Some("working"));
        assert_eq!(runs[1].pid, None);
        assert_eq!(runs[1].status, None);
        assert_eq!(runs[2].pid, None);
        assert_eq!(runs[2].started_at, None);
        assert!(parse_agents_json("no json").is_err());
    }

    #[test]
    fn slug_matches_real_project_dirs() {
        assert_eq!(project_slug(SANDBOX), "-Users-me-Code-nodal-sandbox");
        assert_eq!(
            project_slug("/Users/x/Code/repo/.claude/worktrees/acme-38"),
            "-Users-x-Code-repo--claude-worktrees-acme-38"
        );
        assert!(is_valid_session_id(DONE_SESSION));
        assert!(!is_valid_session_id("../etc"));
        assert!(!is_valid_session_id(""));
    }

    #[test]
    fn finds_session_dir_by_slug_and_by_scan() {
        let projects = fixtures_projects();
        let dir = find_session_dir(&projects, SANDBOX, DONE_SESSION).unwrap();
        assert!(dir.ends_with(DONE_SESSION));
        // cwd that doesn't match the slug: falls back to the scan.
        assert_eq!(find_session_dir(&projects, "/somewhere/else", DONE_SESSION), Some(dir));
        assert_eq!(find_session_dir(&projects, SANDBOX, "no-such-session"), None);
    }

    #[test]
    fn completed_run_uses_final_summary() {
        let dir = fixtures_projects().join(project_slug(SANDBOX)).join(DONE_SESSION);
        let d = read_run_detail(&dir).unwrap();
        assert_eq!(d.source, DetailSource::Final);
        assert_eq!(d.workflow_id, "wf_2450b7a8-254");
        assert_eq!(d.workflow_name.as_deref(), Some("demo-board"));
        assert_eq!(d.status.as_deref(), Some("completed"));
        assert_eq!(d.result_status.as_deref(), Some("green"));
        let titles: Vec<&str> = d.phases.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(titles, ["Idear", "Escribir", "Revisar", "Cerrar"]);
        assert_eq!(d.phases[0].detail.as_deref(), Some("un agente propone 4 subtemas"));
        assert_eq!(d.current_phase.as_deref(), Some("Cerrar"));
        assert_eq!(d.current_phase_index, Some(4));
        assert_eq!(d.agent_count, 10);
        assert_eq!(d.agents.len(), 10);
        assert!(d.agents.iter().all(|a| a.state == AgentState::Done));
        assert_eq!(d.total_tokens, Some(377324));
        assert_eq!(d.total_tool_calls, Some(45));
        assert_eq!(d.duration_ms, Some(117897));
        assert_eq!(d.workflow_count, 1);
        let first = &d.agents[0];
        assert_eq!(first.label, "idear:volcanes");
        assert_eq!(first.agent_id.as_deref(), Some("a6daf84e776bfaad5"));
        assert_eq!(first.model.as_deref(), Some("claude-sonnet-5"));
        assert_eq!(first.phase.as_deref(), Some("Idear"));
        assert_eq!(first.tokens, Some(34612));
        assert_eq!(first.tool_calls, Some(1));
        let last = d.agents.last().unwrap();
        assert_eq!(last.last_tool_name.as_deref(), Some("Bash"));
        assert_eq!(last.last_tool_summary.as_deref(), Some("ls -la ./demo-out && wc -w ./demo-out/*.md"));
    }

    #[test]
    fn cut_run_is_rebuilt_from_journal() {
        let dir = fixtures_projects().join(project_slug(SANDBOX)).join(CUT_SESSION);
        let d = read_run_detail(&dir).unwrap();
        assert_eq!(d.source, DetailSource::Live);
        assert_eq!(d.workflow_id, "wf_4ec5fcf1-d6b");
        assert_eq!(d.workflow_name.as_deref(), Some("demo-board"));
        assert_eq!(d.status, None);
        assert_eq!(d.result_status, None);
        // Phases from the script's meta: the total is known even if they weren't reached.
        let titles: Vec<&str> = d.phases.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(titles, ["Idear", "Escribir", "Revisar", "Cerrar"]);
        assert_eq!(d.current_phase.as_deref(), Some("Escribir"));
        assert_eq!(d.current_phase_index, Some(2));
        assert_eq!(d.agent_count, 5);
        let states: Vec<(&str, AgentState)> = d.agents.iter().map(|a| (a.label.as_str(), a.state)).collect();
        assert_eq!(
            states,
            [
                ("idear:volcanes", AgentState::Done),
                ("escribir:formacion-volcanica", AgentState::Running),
                ("escribir:tipos-de-volcanes", AgentState::Running),
                ("escribir:erupciones-volcanicas", AgentState::Running),
                ("escribir:volcanes-y-el-planeta", AgentState::Running),
            ]
        );
        assert!(d.agents.iter().all(|a| a.model.as_deref() == Some("claude-haiku-4-5-20251001")));
        // Last tool taken from the transcript's tail (only agents in progress).
        let planeta = d.agents.iter().find(|a| a.label == "escribir:volcanes-y-el-planeta").unwrap();
        assert_eq!(planeta.last_tool_name.as_deref(), Some("Bash"));
        assert_eq!(planeta.last_tool_summary.as_deref(), Some("sleep 34"));
        let idear = &d.agents[0];
        assert_eq!(idear.last_tool_name, None);
    }

    #[test]
    fn journal_ignores_unknown_types_and_broken_lines() {
        let text = concat!(
            "{\"type\":\"launched\"}\n",
            "{\"type\":\"started\",\"key\":\"k1\",\"agentId\":\"a1\",\"label\":\"one\",\"phase\":\"A\"}\n",
            "{\"type\":\"something-new\",\"agentId\":\"a1\"}\n",
            "{\"type\":\"started\",\"key\":\"k2\",\"label\":\"two\",\"phase\":\"B\"}\n",
            "{\"type\":\"result\",\"key\":\"k2\",\"result\":null}\n",
            "{\"type\":\"started\",\"key\":\"k3\",\"agentId\":\"a3\",\"lab",
        );
        let agents = parse_journal(text);
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].state, AgentState::Running);
        assert_eq!(agents[1].label, "two");
        assert_eq!(agents[1].state, AgentState::Done);
    }

    #[test]
    fn final_summary_tolerates_garbage() {
        assert!(parse_final("{\"runId\": \"wf_x\", \"workflowProg", "wf_x").is_none());
        let d = parse_final(r#"{"workflowProgress":[{"type":"workflow_phase","index":1,"title":"Only"},
            {"type":"workflow_agent","label":"x","phaseTitle":"Only","state":"exploded","tokens":"lots"}],
            "result":{"status":"purple"}}"#, "wf_y").unwrap();
        assert_eq!(d.workflow_id, "wf_y");
        assert_eq!(d.phases[0].title, "Only");
        assert_eq!(d.agents[0].state, AgentState::Unknown);
        assert_eq!(d.agents[0].tokens, None);
        assert_eq!(d.result_status, None);
    }

    #[test]
    fn script_phases_best_effort() {
        let src = "export const meta = { name: 'x', phases: [ { title: 'One', detail: \"with } brace\" },\n { title: `Two` } ], }";
        let phases = parse_script_phases(src);
        assert_eq!(phases.len(), 2);
        assert_eq!(phases[0].detail.as_deref(), Some("with } brace"));
        assert_eq!(phases[1].title, "Two");
        assert!(parse_script_phases("no phases").is_empty());
    }

    #[test]
    fn agents_json_exposes_blocked_on_permission() {
        // Real entry from a --bg session waiting for approval of a Write (claude 2.1.281).
        let text = r#"[{"pid":21111,"id":"af5deb85","cwd":"/Users/me/Code/nodal-sandbox",
          "kind":"background","startedAt":1790198688952,"sessionId":"af5deb85-3fe1-4ea9-b172-00b68389e167",
          "name":"create perm-test.txt","status":"waiting","waitingFor":"permission prompt","state":"blocked"}]"#;
        let r = &parse_agents_json(text).unwrap()[0];
        assert_eq!(r.state.as_deref(), Some("blocked"));
        assert_eq!(r.status.as_deref(), Some("waiting"));
        assert_eq!(r.waiting_for.as_deref(), Some("permission prompt"));
        assert!(r.is_in_progress());
    }

    #[test]
    fn linear_issue_result_is_typed() {
        let v: Value = serde_json::from_str(
            r#"{"issue":"ACME-4","status":"green","pr":"https://github.com/o/r/pull/3","branch":"feat/acme-4",
            "workdir":"/w/run-acme-4","states":{"inReview":"In Review"},"children":["ACME-35"],"levels":[["ACME-35"]],
            "plan":null,"unmetAcceptance":["Works offline", 3],"nits":["Extract hook"," "]}"#,
        )
        .unwrap();
        let r = parse_result(Some(&v)).unwrap();
        assert_eq!(r.issue.as_deref(), Some("ACME-4"));
        assert_eq!(r.pr.as_deref(), Some("https://github.com/o/r/pull/3"));
        assert_eq!(r.branch.as_deref(), Some("feat/acme-4"));
        assert_eq!(r.workdir.as_deref(), Some("/w/run-acme-4"));
        assert_eq!(r.unmet_acceptance, Some(vec!["Works offline".to_string()]));
        assert_eq!(r.nits, Some(vec!["Extract hook".to_string()]));
        assert!(r.raw.unwrap().contains("\"children\""));

        // Another workflow: no known fields, but with the raw JSON.
        let demo = read_run_detail(&fixtures_projects().join(project_slug(SANDBOX)).join(DONE_SESSION)).unwrap();
        let res = demo.result.unwrap();
        assert_eq!((res.pr, res.unmet_acceptance, res.nits), (None, None, None));
        assert!(res.raw.unwrap().contains("\"tema\": \"volcanes\""));

        // PR that isn't a web URL: dropped. `null` or missing: no result.
        let v: Value = serde_json::from_str(r#"{"pr":"javascript:alert(1)","nits":"not-a-list"}"#).unwrap();
        let r = parse_result(Some(&v)).unwrap();
        assert_eq!((r.pr, r.nits), (None, None));
        assert_eq!(parse_result(Some(&Value::Null)), None);
        assert_eq!(parse_result(None), None);
        assert_eq!(parse_result(Some(&Value::String("done".into()))).unwrap().raw.as_deref(), Some("done"));
    }

    #[test]
    fn transcript_from_real_fixture() {
        let dir = fixtures_projects().join(project_slug(SANDBOX)).join(DONE_SESSION);
        let t = read_agent_transcript(&dir, "wf_2450b7a8-254", "a1007a0f03db17270", 200).unwrap().unwrap();
        assert_eq!(t.label.as_deref(), Some("revisar:volcanes-famosos"));
        assert_eq!(t.model.as_deref(), Some("claude-sonnet-5"));
        assert_eq!(t.phase.as_deref(), Some("Revisar"));
        // Without the harness wrapper and without the indentation.
        let prompt = t.prompt.unwrap();
        assert!(prompt.starts_with("Leé /Users/me/Code/nodal-sandbox/demo-out/04-volcanes-famosos.md"), "{prompt}");
        assert!(!prompt.contains("Workflow harness"));
        // Empty (signed) thinking is skipped: Bash, text, StructuredOutput.
        assert_eq!(t.total_items, 3);
        assert_eq!(t.omitted, 0);
        match &t.items[0] {
            TranscriptItem::ToolUse { name, summary, result, .. } => {
                assert_eq!(name, "Bash");
                assert!(summary.as_deref().unwrap().starts_with("cat /Users/me/Code/nodal-sandbox/demo-out/04"));
                let r = result.as_ref().unwrap();
                assert!(r.text.starts_with("# Volcanes famosos del mundo"));
                assert!(!r.is_error);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(&t.items[1], TranscriptItem::Text { text, .. } if text.starts_with("160 palabras")));
        assert!(t.final_output.unwrap().contains("\"ok\": true"));
        assert!(!t.partial);

        // Limit: only the last N.
        let t = read_agent_transcript(&dir, "wf_2450b7a8-254", "a1007a0f03db17270", 1).unwrap().unwrap();
        assert_eq!((t.items.len(), t.omitted, t.total_items), (1, 2, 3));
        assert!(matches!(&t.items[0], TranscriptItem::ToolUse { name, .. } if name == "StructuredOutput"));

        // No file → None; ids with paths → error.
        assert_eq!(read_agent_transcript(&dir, "wf_2450b7a8-254", "nope", 10).unwrap(), None);
        assert!(read_agent_transcript(&dir, "wf_2450b7a8-254", "../x", 10).is_err());
        assert!(read_agent_transcript(&dir, "..", "a1", 10).is_err());
    }

    #[test]
    fn transcript_tolerates_errors_arrays_and_long_text() {
        let long = "x".repeat(TEXT_MAX + 50);
        let lines = [
            r#"{"type":"user","message":{"role":"user","content":"Task without wrapper"}}"#.to_string(),
            r#"{"type":"attachment","attachment":{"type":"skill_listing","content":"user assistant"}}"#.to_string(),
            format!(r#"{{"type":"assistant","message":{{"content":[{{"type":"thinking","thinking":"hmm"}},{{"type":"text","text":"{long}"}}]}}}}"#),
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/a/b.rs"}}]}}"#.to_string(),
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":[{"type":"text","text":"File not found"},{"type":"image"}]}]}}"#.to_string(),
            r#"{"type":"user","message":{"content":"still there?"}}"#.to_string(),
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t2","name":"Bash","input":{}}]}}"#.to_string(),
            r#"{"type":"assistant","message":{"content":[{"type":"text","te"#.to_string(),
        ];
        let p = parse_transcript(&lines.join("\n"), 100);
        assert_eq!(p.prompt.as_deref(), Some("Task without wrapper"));
        assert_eq!(p.total, 5);
        assert!(matches!(&p.items[0], TranscriptItem::Thinking { text, .. } if text == "hmm"));
        assert!(matches!(&p.items[1], TranscriptItem::Text { truncated: true, text } if text.chars().count() == TEXT_MAX + 1));
        match &p.items[2] {
            TranscriptItem::ToolUse { summary, result: Some(r), .. } => {
                assert_eq!(summary.as_deref(), Some("/a/b.rs"));
                assert!(r.is_error);
                assert_eq!(r.text, "File not found\n[image]");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(&p.items[3], TranscriptItem::User { text, .. } if text == "still there?"));
        // Tool with no result yet (agent running) and empty input.
        assert!(matches!(&p.items[4], TranscriptItem::ToolUse { result: None, input: None, summary: None, .. }));
        // No StructuredOutput: the final output is the last text.
        assert!(p.final_output.unwrap().starts_with("xxx"));
    }

    #[test]
    fn path_ids() {
        assert!(is_valid_path_id("wf_2450b7a8-254"));
        assert!(is_valid_path_id("a1007a0f03db17270"));
        for bad in ["", "..", "a/b", "a\\b", "-rf", "wf_x.json", "a b"] {
            assert!(!is_valid_path_id(bad), "{bad}");
        }
    }

    #[test]
    fn session_without_workflows_has_no_detail() {
        let tmp = std::env::temp_dir().join(format!("nodal-runs-test-{}", std::process::id()));
        fs::create_dir_all(&tmp).unwrap();
        assert_eq!(read_run_detail(&tmp), None);
        let _ = fs::remove_dir_all(&tmp);
    }

    /// Every workflow transcript on this machine: none fails, and timings are measured.
    #[test]
    #[ignore]
    fn real_transcripts_on_disk() {
        let projects = claude_config_dir().unwrap().join("projects");
        let (mut n, mut slowest) = (0, (std::time::Duration::ZERO, PathBuf::new()));
        for proj in fs::read_dir(&projects).unwrap().flatten() {
            for sess in fs::read_dir(proj.path()).into_iter().flatten().flatten() {
                let wfs = sess.path().join("subagents").join("workflows");
                for wf in fs::read_dir(&wfs).into_iter().flatten().flatten() {
                    let wf_id = wf.file_name().to_string_lossy().into_owned();
                    for f in fs::read_dir(wf.path()).into_iter().flatten().flatten() {
                        let name = f.file_name().to_string_lossy().into_owned();
                        let Some(agent) = name.strip_prefix("agent-").and_then(|s| s.strip_suffix(".jsonl")) else { continue };
                        let t0 = std::time::Instant::now();
                        let t = read_agent_transcript(&sess.path(), &wf_id, agent, 200).unwrap().unwrap();
                        let dt = t0.elapsed();
                        assert!(t.items.len() <= 200);
                        if dt > slowest.0 {
                            slowest = (dt, f.path());
                        }
                        n += 1;
                    }
                }
            }
        }
        eprintln!("{n} transcripts; slowest {:?} {}", slowest.0, slowest.1.display());
    }

    /// Against this machine's real data:
    /// `NODAL_SANDBOX=<sandbox repo path> cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn real_sessions_on_disk() {
        let sandbox = std::env::var("NODAL_SANDBOX").expect("NODAL_SANDBOX=<sandbox repo path>");
        let projects = claude_config_dir().unwrap().join("projects");
        for session in [DONE_SESSION, CUT_SESSION] {
            let dir = find_session_dir(&projects, &sandbox, session).expect("real session not found");
            let d = read_run_detail(&dir).expect("no workflow");
            eprintln!("{session}: {d:#?}");
        }
    }

    #[test]
    fn session_transcript_skips_sidechains() {
        let lines = [
            r#"{"type":"user","message":{"role":"user","content":"Fix the login bug"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Looking at the code."}]}}"#,
            r#"{"type":"assistant","isSidechain":true,"message":{"content":[{"type":"text","text":"subagent"}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"a.rs"}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Done."}]}}"#,
        ];
        let dir = std::env::temp_dir().join(format!("nodal-session-tx-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.jsonl");
        fs::write(&path, lines.join("\n")).unwrap();
        let t = read_session_transcript(&path, "run-1", Some("Claude".into()), None, 2).unwrap().unwrap();
        assert_eq!(t.agent_id, "run-1");
        assert_eq!(t.prompt.as_deref(), Some("Fix the login bug"));
        assert_eq!((t.total_items, t.omitted), (3, 1));
        assert_eq!(t.final_output.as_deref(), Some("Done."));
        assert!(!t.items.iter().any(|i| matches!(i, TranscriptItem::Text { text, .. } if text == "subagent")));
        assert!(read_session_transcript(&dir.join("nope.jsonl"), "x", None, None, 10).unwrap().is_none());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn usage_tokens_dedupe_by_message_id() {
        let lines = [
            r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"a"}],"usage":{"input_tokens":10,"output_tokens":5,"cache_creation_input_tokens":100,"cache_read_input_tokens":1000}}}"#,
            r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"t","name":"Bash","input":{}}],"usage":{"input_tokens":10,"output_tokens":5,"cache_creation_input_tokens":100,"cache_read_input_tokens":1000}}}"#,
            r#"{"type":"user","message":{"content":"usage"}}"#,
            r#"{"type":"assistant","isSidechain":true,"message":{"id":"m2","content":[],"usage":{"input_tokens":1,"output_tokens":2}}}"#,
            "not json \"usage\" \"assistant\"",
        ];
        assert_eq!(usage_tokens_in(lines), Some(1115 + 3));
        assert_eq!(usage_tokens_in([r#"{"type":"assistant","message":{"content":[]}}"#]), None);
        let dir = std::env::temp_dir().join(format!("nodal-usage-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.jsonl");
        fs::write(&path, lines.join("\n")).unwrap();
        assert_eq!(read_usage_tokens(&path), Some(1118));
        assert_eq!(read_usage_tokens(&dir.join("nope.jsonl")), None);
        fs::remove_dir_all(&dir).ok();
    }
}
