//! WARNING: Claude Code's internal, UNDOCUMENTED format (observed in v2.1.280/281).
//!
//! Everything this module needs to know about how Claude Code stores its sessions lives
//! here and only here (the rest, in `runs::claude_fs`):
//! - `claude agents --json --all`: list with `kind` "interactive" | "background".
//!   Interactive: `pid`, `status` ("busy" | "idle" | "waiting"), `waitingFor`, no `id`
//!   or `state`. Background: short `id`, `state` ("working" | "blocked" | "done" |
//!   "stopped"); `pid`/`status` only while the process is alive.
//! - `~/.claude/projects/<slug(cwd)>/<sessionId>.jsonl`: the session transcript. Each
//!   line carries `cwd` (the current directory at that moment), `timestamp`, `entrypoint`
//!   ("cli", "sdk-cli", …).
//! - `~/.claude/projects/<slug>/<sessionId>/subagents/agent-<agentId>.jsonl` (+ `.meta.json`):
//!   subagents launched with the Agent tool. Workflow ones, in
//!   `subagents/workflows/<wf_id>/agent-<agentId>.jsonl`.
//!   `meta.json`: `agentType`, `description`, `worktreePath` (if a worktree was created),
//!   `inheritedWorktreePath` (child of an agent with a worktree), `parentAgentId`,
//!   `workflowPhase`, `model`.
//! - A subagent is finished when its last line is an `assistant` with
//!   `message.stop_reason == "end_turn"`; while it works, the last line is an `assistant`
//!   block with `stop_reason: null` (thinking/text/tool_use) or a `user` with
//!   `tool_result`. If it's resumed (SendMessage), new lines get appended and it becomes
//!   active again.
//!
//! Policy: tolerate anything unknown and degrade to `None` instead of failing.
//! Transcripts weigh MBs: only the tail is read.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::runs::claude_fs;

fn lenient<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let v = Value::deserialize(d)?;
    Ok(serde_json::from_value(v).ok())
}

// ---------------------------------------------------------------------------
// `claude agents --json --all`
// ---------------------------------------------------------------------------

/// A session as reported by `claude agents` (interactive or background).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSession {
    #[serde(default, deserialize_with = "lenient")]
    pub id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub session_id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub kind: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub cwd: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub pid: Option<u32>,
    #[serde(default, deserialize_with = "lenient")]
    pub status: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub state: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub waiting_for: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub started_at: Option<f64>,
}

impl AgentSession {
    /// Live process (`pid`), or a background session that `claude agents` reports as active.
    pub fn alive(&self) -> bool {
        self.pid.is_some() || matches!(self.state.as_deref(), Some("working") | Some("blocked"))
    }
}

/// All sessions with a valid `sessionId`; odd entries are dropped.
pub fn parse_agents(text: &str) -> Result<Vec<AgentSession>, String> {
    let values: Vec<Value> = serde_json::from_str(text.trim())
        .map_err(|e| format!("`claude agents --json` didn't return the expected JSON list: {e}"))?;
    Ok(values
        .into_iter()
        .filter_map(|v| serde_json::from_value::<AgentSession>(v).ok())
        .filter(|a| a.session_id.as_deref().is_some_and(claude_fs::is_valid_session_id))
        .collect())
}

// ---------------------------------------------------------------------------
// Session files
// ---------------------------------------------------------------------------

pub fn mtime_ms(path: &Path) -> Option<i64> {
    let t = fs::metadata(path).ok()?.modified().ok()?;
    Some(t.duration_since(UNIX_EPOCH).ok()?.as_millis() as i64)
}

/// `<projects>/<slug>/<sessionId>.jsonl`; if the slug doesn't match, looks in any project.
pub fn find_session_jsonl(projects: &Path, cwd: Option<&str>, session_id: &str) -> Option<PathBuf> {
    let file = format!("{session_id}.jsonl");
    if let Some(cwd) = cwd {
        let direct = projects.join(claude_fs::project_slug(cwd)).join(&file);
        if direct.is_file() {
            return Some(direct);
        }
    }
    fs::read_dir(projects).ok()?.flatten().map(|e| e.path().join(&file)).find(|p| p.is_file())
}

/// Last `window` bytes of the file, without the first line if it was cut off.
pub fn read_tail(path: &Path, window: u64) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(window);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    file.take(window).read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    Some(if start > 0 { text.split_once('\n').map_or(String::new(), |(_, r)| r.to_string()) } else { text })
}

/// What's extracted from a transcript's tail.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct TailInfo {
    /// The last entry is an `assistant` with `stop_reason` "end_turn"/"stop_sequence".
    pub finished: bool,
    /// There was at least one conversation line (user/assistant) in the tail.
    pub has_turns: bool,
    /// `cwd` of the most recent line that has one.
    pub cwd: Option<String>,
    pub entrypoint: Option<String>,
    pub last_tool: Option<String>,
    pub last_tool_summary: Option<String>,
    /// Absolute paths touched by the tools in the tail (`file_path`, `path`,
    /// `notebook_path`): an agent without a worktree that didn't `cd` may still be
    /// editing the repo through absolute paths.
    pub touched_paths: Vec<String>,
}

const MAX_TOUCHED: usize = 32;

fn collect_touched(v: &Value, out: &mut Vec<String>) {
    let Some(content) = v.get("message").and_then(|m| m.get("content")).and_then(Value::as_array) else { return };
    for c in content {
        if c.get("type").and_then(Value::as_str) != Some("tool_use") {
            continue;
        }
        let Some(input) = c.get("input") else { continue };
        for key in ["file_path", "path", "notebook_path"] {
            if let Some(p) = input.get(key).and_then(Value::as_str).filter(|p| p.starts_with('/')) {
                if out.len() < MAX_TOUCHED && !out.iter().any(|x| x == p) {
                    out.push(p.to_string());
                }
            }
        }
    }
}

pub fn parse_tail(text: &str) -> TailInfo {
    let mut info = TailInfo::default();
    let mut decided = false;
    for line in text.lines().rev() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if line.contains("\"tool_use\"") {
            collect_touched(&v, &mut info.touched_paths);
        }
        if info.cwd.is_none() {
            info.cwd = v.get("cwd").and_then(Value::as_str).map(String::from);
        }
        if info.entrypoint.is_none() {
            info.entrypoint = v.get("entrypoint").and_then(Value::as_str).map(String::from);
        }
        if !decided {
            match v.get("type").and_then(Value::as_str) {
                Some("assistant") => {
                    decided = true;
                    info.has_turns = true;
                    let stop = v.get("message").and_then(|m| m.get("stop_reason")).and_then(Value::as_str);
                    info.finished = matches!(stop, Some("end_turn") | Some("stop_sequence"));
                }
                Some("user") => {
                    decided = true;
                    info.has_turns = true;
                }
                // queue-operation, attachment, system, … don't say whether it finished.
                _ => {}
            }
        }
    }
    if let Some((name, summary)) = claude_fs::last_tool_in_lines(text) {
        info.last_tool = Some(name);
        info.last_tool_summary = summary;
    }
    info
}

#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentMeta {
    #[serde(default, deserialize_with = "lenient")]
    pub agent_type: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub description: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub worktree_path: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub inherited_worktree_path: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub parent_agent_id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub workflow_phase: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub model: Option<String>,
}

pub fn parse_meta(text: &str) -> SubagentMeta {
    serde_json::from_str(text).unwrap_or_default()
}

/// A subagent transcript found on disk.
#[derive(Debug, Clone, PartialEq)]
pub struct SubagentFile {
    pub agent_id: String,
    pub transcript: PathBuf,
    pub meta: Option<PathBuf>,
    /// `wf_…` if it's a workflow agent.
    pub workflow_id: Option<String>,
}

fn agent_files_in(dir: &Path, workflow_id: Option<&str>, out: &mut Vec<SubagentFile>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_prefix("agent-").and_then(|n| n.strip_suffix(".jsonl")) else { continue };
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            continue;
        }
        let meta = dir.join(format!("agent-{id}.meta.json"));
        out.push(SubagentFile {
            agent_id: id.to_string(),
            transcript: e.path(),
            meta: meta.is_file().then_some(meta),
            workflow_id: workflow_id.map(String::from),
        });
    }
}

/// A session's `subagents/agent-*.jsonl` and `subagents/workflows/<wf>/agent-*.jsonl`.
pub fn subagent_files(session_dir: &Path) -> Vec<SubagentFile> {
    let root = session_dir.join("subagents");
    let mut out = Vec::new();
    agent_files_in(&root, None, &mut out);
    if let Ok(wfs) = fs::read_dir(root.join("workflows")) {
        for wf in wfs.flatten().filter(|e| e.path().is_dir()) {
            let wf_id = wf.file_name().to_string_lossy().into_owned();
            agent_files_in(&wf.path(), Some(&wf_id), &mut out);
        }
    }
    out
}

/// Session transcripts (`<sessionId>.jsonl`) of the projects whose slug starts with
/// `slug_prefix` (the repo and its worktrees in `.claude/worktrees/…`), modified since
/// `since_ms`. The prefix may catch sibling repos (`nodal-sandbox`): the caller
/// filters by the transcript's real `cwd`.
pub fn recent_session_transcripts(projects: &Path, slug_prefix: &str, since_ms: i64) -> Vec<(String, PathBuf, i64)> {
    let Ok(dirs) = fs::read_dir(projects) else { return Vec::new() };
    let mut out = Vec::new();
    for d in dirs.flatten() {
        if !d.file_name().to_string_lossy().starts_with(slug_prefix) {
            continue;
        }
        let Ok(files) = fs::read_dir(d.path()) else { continue };
        for f in files.flatten() {
            let name = f.file_name().to_string_lossy().into_owned();
            let Some(sid) = name.strip_suffix(".jsonl") else { continue };
            if !claude_fs::is_valid_session_id(sid) {
                continue;
            }
            let path = f.path();
            if let Some(m) = mtime_ms(&path).filter(|&m| m >= since_ms) {
                out.push((sid.to_string(), path, m));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/activity/fixtures")
    }

    #[test]
    fn agents_real_format_all_kinds() {
        let text = fs::read_to_string(fixtures().join("agents.json")).unwrap();
        let a = parse_agents(&text).unwrap();
        assert_eq!(a.len(), 5, "the entry without sessionId is dropped");
        let coord = a.iter().find(|x| x.kind.as_deref() == Some("interactive")).unwrap();
        assert_eq!(coord.id, None);
        assert_eq!(coord.status.as_deref(), Some("busy"));
        assert!(coord.alive());
        let blocked = a.iter().find(|x| x.state.as_deref() == Some("blocked")).unwrap();
        assert_eq!(blocked.waiting_for.as_deref(), Some("permission prompt"));
        assert!(blocked.alive());
        let done = a.iter().find(|x| x.id.as_deref() == Some("43495c2e")).unwrap();
        assert!(!done.alive());
        assert!(parse_agents("nope").is_err());
        assert_eq!(parse_agents("[]").unwrap(), vec![]);
    }

    #[test]
    fn tail_detects_finished_running_and_tool() {
        let dir = fixtures().join("projects/-Users-me-Code/11111111-2222-3333-4444-555555555555/subagents");
        let done = parse_tail(&fs::read_to_string(dir.join("agent-adone00000000001.jsonl")).unwrap());
        assert!(done.finished && done.has_turns);
        assert_eq!(done.cwd.as_deref(), Some("/Users/me/Code"));
        let live = parse_tail(&fs::read_to_string(dir.join("agent-alive0000000001.jsonl")).unwrap());
        assert!(!live.finished && live.has_turns);
        assert_eq!(live.last_tool.as_deref(), Some("Bash"));
        assert_eq!(live.last_tool_summary.as_deref(), Some("cargo test"));
        assert_eq!(live.cwd.as_deref(), Some("/Users/me/Code/repo/.claude/worktrees/agent-alive0000000001"));
        assert_eq!(done.touched_paths, vec!["/Users/me/Code/x.rs".to_string()]);
        // Tail cut mid-line plus garbage: doesn't break.
        let t = parse_tail("{\"type\":\"assi\n{bad json\n");
        assert_eq!(t, TailInfo::default());
    }

    #[test]
    fn meta_is_lenient() {
        let m = parse_meta(r#"{"agentType":"general-purpose","worktreePath":"/r/.claude/worktrees/a","spawnDepth":1,"description":42}"#);
        assert_eq!(m.agent_type.as_deref(), Some("general-purpose"));
        assert_eq!(m.worktree_path.as_deref(), Some("/r/.claude/worktrees/a"));
        assert_eq!(m.description, None);
        assert_eq!(parse_meta("garbage"), SubagentMeta::default());
    }

    #[test]
    fn finds_subagents_including_workflow_agents() {
        let dir = fixtures().join("projects/-Users-me-Code-repo/99999999-0000-0000-0000-000000000001");
        let mut files = subagent_files(&dir);
        files.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
        let ids: Vec<_> = files.iter().map(|f| (f.agent_id.as_str(), f.workflow_id.as_deref(), f.meta.is_some())).collect();
        assert_eq!(ids, vec![("aworkflow000000001", Some("wf_1234"), true)]);
    }

    #[test]
    fn tail_reads_only_the_end() {
        let tmp = std::env::temp_dir().join(format!("nodal-activity-tail-{}.jsonl", std::process::id()));
        let mut s = String::new();
        for i in 0..2000 {
            s.push_str(&format!("{{\"type\":\"user\",\"n\":{i}}}\n"));
        }
        fs::write(&tmp, &s).unwrap();
        let tail = read_tail(&tmp, 256).unwrap();
        assert!(tail.len() <= 256);
        assert!(tail.lines().all(|l| l.starts_with('{') && l.ends_with('}')), "{tail}");
        assert!(tail.contains("\"n\":1999"));
        fs::remove_file(&tmp).unwrap();
    }
}
