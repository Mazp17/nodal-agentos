//! Detail of a session's workflow run: the final summary written to `workflows/wf_*.json`
//! when it's done, or rebuilt live from the journal while it's still running.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Deserialize;
use serde_json::Value;

use nodal_domain::model::claude::{
    AgentInfo, AgentState, DetailSource, PhaseInfo, RunDetail, RunResult,
};

use super::js_script::parse_script_phases;
use super::last_tool::last_tool_from_transcript;
use super::{lenient, truncate};

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

pub(crate) fn wf_live_dir(session_dir: &Path, wf_id: &str) -> PathBuf {
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
    phases
        .iter()
        .position(|p| p.title == title)
        .map(|i| i as u32 + 1)
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
        .filter_map(|p| {
            Some(PhaseInfo {
                title: p.title?,
                detail: p.detail,
            })
        })
        .collect();
    if phases.is_empty() {
        phases = progress
            .iter()
            .filter(|p| p.kind.as_deref() == Some("workflow_phase"))
            .filter_map(|p| {
                Some(PhaseInfo {
                    title: p.title.clone()?,
                    detail: None,
                })
            })
            .collect();
    }

    let agents: Vec<AgentInfo> = progress
        .into_iter()
        .filter(|p| p.kind.as_deref() == Some("workflow_agent"))
        .map(|p| AgentInfo {
            state: map_state(p.state.as_deref()),
            label: p
                .label
                .or_else(|| p.agent_id.clone())
                .unwrap_or_else(|| "(unnamed)".into()),
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
        let Ok(entry) = serde_json::from_str::<RawJournalLine>(line) else {
            continue;
        };
        let Some(ident) = entry.agent_id.clone().or(entry.key.clone()) else {
            continue;
        };
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
        let Some(id) = a.agent_id.clone() else {
            continue;
        };
        a.model = fs::read_to_string(dir.join(format!("agent-{id}.meta.json")))
            .ok()
            .and_then(|t| serde_json::from_str::<RawMeta>(&t).ok())
            .and_then(|m| m.model);
        if a.state == AgentState::Running {
            if let Some((name, summary)) =
                last_tool_from_transcript(&dir.join(format!("agent-{id}.jsonl")))
            {
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
                phases.push(PhaseInfo {
                    title: ph.clone(),
                    detail: None,
                });
            }
        }
    }
    let workflow_name = script.as_ref().and_then(|p| {
        let file = p.file_name()?.to_string_lossy().into_owned();
        file.strip_suffix(&format!("-{wf_id}.js"))
            .map(str::to_string)
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
        .find(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().ends_with(&suffix))
        })
}

// --- Workflow result ----------------------------------------------------------

const RESULT_RAW_MAX: usize = 8000;
const RESULT_LIST_MAX: usize = 100;
const RESULT_ITEM_MAX: usize = 2000;

fn str_field(obj: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key)?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| truncate(s, RESULT_ITEM_MAX))
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
    let mut out = RunResult {
        raw: Some(truncate(&raw, RESULT_RAW_MAX)),
        ..RunResult::default()
    };
    if let Some(obj) = v.as_object() {
        out.issue = str_field(obj, "issue");
        // Web URLs only: the frontend opens it with the system opener.
        // Not clipped: a cut URL would open a broken link.
        out.pr = obj
            .get("pr")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|u| {
                (u.starts_with("https://") || u.starts_with("http://"))
                    && u.len() <= RESULT_ITEM_MAX
            })
            .map(String::from);
        out.branch = str_field(obj, "branch");
        out.workdir = str_field(obj, "workdir");
        out.where_ = str_field(obj, "where");
        out.unmet_acceptance = str_list(obj, "unmetAcceptance");
        out.nits = str_list(obj, "nits");
    }
    Some(out)
}

#[cfg(test)]
mod tests;
