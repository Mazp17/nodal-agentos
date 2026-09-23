//! ATENCIÓN: formato interno y SIN DOCUMENTAR de Claude Code (observado en v2.1.281).
//!
//! Todo lo que depende de cómo Claude Code imprime su salida o escribe sus archivos vive
//! acá y solo acá, para que cuando cambie haya un único lugar que tocar:
//! - salida de `claude --bg` (línea `backgrounded · <id>`),
//! - salida de `claude agents --json --all`,
//! - layout de `~/.claude/projects/<slug>/<sessionId>/` (journal, meta, transcripts,
//!   resumen final `workflows/wf_*.json`, script `workflows/scripts/*-wf_*.js`).
//!
//! Criterio: tolerar todo lo desconocido (campos nuevos, tipos inesperados, líneas
//! cortadas a mitad de escritura) y degradar a `None` en vez de fallar.

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

/// Deserializa un campo opcional sin fallar si el tipo no es el esperado.
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

/// Extrae el id corto de una línea como `backgrounded · ddb91222` (con o sin colores ANSI).
pub fn parse_bg_line(line: &str) -> Option<String> {
    let clean = strip_ansi(line);
    let pos = clean.find("backgrounded")?;
    let rest = &clean[pos + "backgrounded".len()..];
    // Primer token después de `backgrounded` (puede venir texto detrás del id).
    let id = rest
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .find(|t| !t.is_empty())?;
    is_plausible_id(id).then(|| id.to_string())
}

/// Plan B si la salida no trae la palabra `backgrounded` (p.ej. otro formato sin TTY):
/// aceptar una salida que sea solo un id hexadecimal. No observado, solo defensivo.
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
                // CSI: parámetros hasta una letra final.
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

/// Sesiones `kind == "background"`, más recientes primero. Entradas sin `id` o
/// `sessionId` se descartan (las interactivas, por ejemplo, no traen `id`).
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
    runs.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    Ok(runs)
}

// ---------------------------------------------------------------------------
// Ubicación de la sesión en disco
// ---------------------------------------------------------------------------

/// `$CLAUDE_CONFIG_DIR` o `~/.claude`.
pub fn claude_config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".claude"))
}

/// Slug del directorio de proyecto: todo carácter no alfanumérico pasa a `-`.
/// Verificado contra `~/.claude/projects`: `/Users/x/Code/nodal-sandbox` →
/// `-Users-x-Code-nodal-sandbox`, y `/.claude/worktrees` → `--claude-worktrees`.
pub fn project_slug(cwd: &str) -> String {
    cwd.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

/// Un sessionId viene del frontend y termina en una ruta: solo `[A-Za-z0-9-]`.
pub fn is_valid_session_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// `<projects>/<slug>/<sessionId>`. Si el slug no coincide (rutas largas o reglas que
/// cambien), busca la sesión en cualquier proyecto.
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
// Detalle de un workflow
// ---------------------------------------------------------------------------

/// Detalle del workflow más reciente de la sesión, o `None` si la sesión no lanzó ninguno.
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
        // Sin resumen (o a medio escribir): se reconstruye desde el journal.
        None => read_live(session_dir, &wf_id),
    };
    detail.workflow_count = workflow_count;
    Some(detail)
}

fn wf_live_dir(session_dir: &Path, wf_id: &str) -> PathBuf {
    session_dir.join("subagents").join("workflows").join(wf_id)
}

/// Ids `wf_*` presentes en `subagents/workflows/` (dirs) y `workflows/` (resúmenes).
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

// --- Resumen final: workflows/wf_<id>.json --------------------------------

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

/// `None` si el archivo no es JSON válido (p.ej. a medio escribir).
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

// --- En vivo: journal.jsonl + meta + transcripts ------------------------------

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

/// Agentes del journal en orden de arranque: `started` sin `result` = corriendo.
/// Ignora tipos desconocidos y líneas ilegibles (la última puede estar a medio escribir).
pub fn parse_journal(text: &str) -> Vec<AgentInfo> {
    let mut agents: Vec<AgentInfo> = Vec::new();
    // Identidad de cada agente: agentId, o la `key` si faltara.
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
                // Un reintento puede reusar la identidad: queda el último arranque.
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
    // Lossy: la última línea puede estar cortada a mitad de un carácter multibyte.
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
    // Sin script legible: las fases que se vieron en el journal, en orden.
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

/// `workflows/scripts/<nombre>-<wf_id>.js`.
fn find_script(session_dir: &Path, wf_id: &str) -> Option<PathBuf> {
    let suffix = format!("-{wf_id}.js");
    fs::read_dir(session_dir.join("workflows").join("scripts"))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().ends_with(&suffix)))
}

/// Fases declaradas en `meta.phases` del script JS del workflow. Best effort: es JS, no
/// JSON; se buscan `title:` / `detail:` con literales de string dentro del array.
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

/// Índice del cierre que balancea, saltando literales de string.
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

/// Trozos `{ ... }` de primer nivel, sin las llaves.
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

/// Valor de `key: '...'` (comillas simples, dobles o backticks).
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

// --- Transcript de un agente: última tool ----------------------------------

/// Última `tool_use` de un transcript `agent-<id>.jsonl`. Lee solo la cola del archivo
/// (pueden pesar MB): primero 64 KB, y si ahí no hay ninguna, 1 MB.
pub fn last_tool_from_transcript(path: &Path) -> Option<(String, Option<String>)> {
    let mut file = fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    for window in [64 * 1024u64, 1024 * 1024] {
        let start = len.saturating_sub(window);
        file.seek(SeekFrom::Start(start)).ok()?;
        let mut buf = Vec::new();
        file.by_ref().take(window).read_to_end(&mut buf).ok()?;
        let text = String::from_utf8_lossy(&buf);
        // Si no arrancamos al principio, la primera línea está cortada.
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
        // Filtro barato antes de parsear líneas que pueden ser enormes.
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

/// Recorta a `max` caracteres (con `…`) e indica si recortó.
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

// --- Resultado del workflow ---------------------------------------------------

const RESULT_RAW_MAX: usize = 8000;
const RESULT_LIST_MAX: usize = 100;
const RESULT_ITEM_MAX: usize = 2000;

fn str_field(obj: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key)?.as_str().map(str::trim).filter(|s| !s.is_empty()).map(|s| truncate(s, RESULT_ITEM_MAX))
}

/// Lista de strings; ignora elementos que no lo sean. `None` si el campo no es un array.
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

/// Campos conocidos del `result` (forma de `linear-issue`) más el JSON crudo recortado.
/// `result` es libre: lo que no tenga la forma esperada queda en `None`.
pub fn parse_result(result: Option<&Value>) -> Option<RunResult> {
    let v = result.filter(|v| !v.is_null())?;
    let raw = match v {
        Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    };
    let mut out = RunResult { raw: Some(truncate(&raw, RESULT_RAW_MAX)), ..RunResult::default() };
    if let Some(obj) = v.as_object() {
        out.issue = str_field(obj, "issue");
        // Solo URLs web: el frontend la abre con el opener del sistema.
        out.pr = str_field(obj, "pr").filter(|u| u.starts_with("https://") || u.starts_with("http://"));
        out.branch = str_field(obj, "branch");
        out.workdir = str_field(obj, "workdir");
        out.where_ = str_field(obj, "where");
        out.unmet_acceptance = str_list(obj, "unmetAcceptance");
        out.nits = str_list(obj, "nits");
    }
    Some(out)
}

// --- Transcript completo de un subagente ---------------------------------------

/// Por encima de esto se lee el principio (prompt) y la cola (conversación reciente).
const TRANSCRIPT_MAX_READ: u64 = 16 * 1024 * 1024;
const TRANSCRIPT_HEAD: u64 = 1024 * 1024;
const PROMPT_MAX: usize = 8000;
const TEXT_MAX: usize = 4000;
const TOOL_INPUT_MAX: usize = 2000;
const TOOL_RESULT_MAX: usize = 1500;
const SUMMARY_MAX: usize = 160;
pub const TRANSCRIPT_DEFAULT_LIMIT: u32 = 200;
pub const TRANSCRIPT_MAX_LIMIT: u32 = 2000;

/// Ids que terminan en una ruta (`wf_...`, id de agente): solo `[A-Za-z0-9_-]`.
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

/// Lo que sale de parsear el texto del transcript.
#[derive(Debug, Default)]
pub struct ParsedTranscript {
    pub prompt: Option<String>,
    pub items: Vec<TranscriptItem>,
    pub total: usize,
    pub final_output: Option<String>,
}

/// El workflow envuelve la tarea: `[Workflow harness — computed task] ... follows:` y el
/// texto indentado con dos espacios. Devuelve el texto sin el marco.
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

/// Parsea las líneas de un `agent-<id>.jsonl`: prompt inicial, conversación con cada
/// `tool_result` pegado a su `tool_use`, y salida final. Devuelve como mucho los últimos
/// `limit` items. Tolera líneas rotas y tipos desconocidos.
pub fn parse_transcript(text: &str, limit: usize) -> ParsedTranscript {
    let mut prompts: Vec<String> = Vec::new();
    let mut items: Vec<TranscriptItem> = Vec::new();
    let mut seen_assistant = false;
    let mut last_text: Option<String> = None;
    let mut structured: Option<String> = None;

    for line in text.lines() {
        // Filtro barato: los adjuntos (listas de skills, snapshots) son la mayor parte del archivo.
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
                                // El tool_use suele estar muy cerca: se busca desde el final.
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

/// Lee el archivo entero, o si es muy grande su principio (donde está el prompt) y su
/// cola. Devuelve `(texto, parcial, bytes)`.
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
    // Solo líneas completas del principio.
    let head = head.rsplit_once('\n').map_or("", |(h, _)| h);
    let tail_len = TRANSCRIPT_MAX_READ - TRANSCRIPT_HEAD;
    file.seek(SeekFrom::Start(len - tail_len))?;
    let mut tail = Vec::new();
    file.take(tail_len).read_to_end(&mut tail)?;
    let tail = String::from_utf8_lossy(&tail);
    let tail = tail.split_once('\n').map_or("", |(_, r)| r);
    Ok((format!("{head}\n{tail}"), true, len))
}

/// Transcript del agente `agent_id` del workflow `wf_id`. `Ok(None)` si no hay archivo.
/// Los ids tienen que venir validados con `is_valid_path_id`.
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

// ---------------------------------------------------------------------------
// Tests: fixtures recortados de runs reales en `src/runs/fixtures/`.
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
    fn bg_line_real_format() {
        assert_eq!(parse_bg_line("backgrounded · ddb91222").as_deref(), Some("ddb91222"));
        assert_eq!(parse_bg_line("\u{1b}[2mbackgrounded\u{1b}[0m · \u{1b}[1mddb91222\u{1b}[0m").as_deref(), Some("ddb91222"));
        assert_eq!(parse_bg_line("  backgrounded · ab12cd34  \r").as_deref(), Some("ab12cd34"));
        assert_eq!(parse_bg_line("backgrounded · ab12cd34 · run `claude agents` to view").as_deref(), Some("ab12cd34"));
        assert_eq!(parse_bg_line("backgrounded"), None);
        assert_eq!(parse_bg_line("Error: not logged in"), None);
        assert_eq!(parse_bare_id("ddb91222\n").as_deref(), Some("ddb91222"));
        assert_eq!(parse_bare_id("hola mundo"), None);
    }

    #[test]
    fn agents_json_filters_background_and_tolerates_missing_fields() {
        let text = r#"[
          {"id":"0eef7f11","cwd":"/a","kind":"background","startedAt":1787690543980,
           "sessionId":"0eef7f11-d932-4001-97f8-df01555801d7","name":"viejo","state":"done"},
          {"pid":59575,"cwd":"/b","kind":"interactive","startedAt":1790087661575,
           "sessionId":"a5ff54f0-6f8e-49b4-9213-0c859d3432de","name":"interactiva","status":"idle"},
          {"pid":123,"id":"ddb91222","cwd":"/c","kind":"background","startedAt":1790192548000,
           "sessionId":"ddb91222-57b2-4ae4-a0bb-d1c5993dc1c8","name":"nuevo","status":"busy",
           "state":"working","campoNuevo":{"x":1}},
          {"id":"sinsesion","kind":"background"},
          {"id":"raro","sessionId":"s","kind":"background","pid":"no-es-numero","startedAt":"ayer"}
        ]"#;
        let runs = parse_agents_json(text).unwrap();
        let ids: Vec<&str> = runs.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["ddb91222", "0eef7f11", "raro"]);
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
        // cwd que no coincide con el slug: cae al escaneo.
        assert_eq!(find_session_dir(&projects, "/otro/lado", DONE_SESSION), Some(dir));
        assert_eq!(find_session_dir(&projects, SANDBOX, "no-existe"), None);
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
        // Fases del meta del script: el total se conoce aunque no se hayan alcanzado.
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
        // Última tool sacada de la cola del transcript (solo agentes en curso).
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
            "{\"type\":\"started\",\"key\":\"k1\",\"agentId\":\"a1\",\"label\":\"uno\",\"phase\":\"A\"}\n",
            "{\"type\":\"algo-nuevo\",\"agentId\":\"a1\"}\n",
            "{\"type\":\"started\",\"key\":\"k2\",\"label\":\"dos\",\"phase\":\"B\"}\n",
            "{\"type\":\"result\",\"key\":\"k2\",\"result\":null}\n",
            "{\"type\":\"started\",\"key\":\"k3\",\"agentId\":\"a3\",\"lab",
        );
        let agents = parse_journal(text);
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].state, AgentState::Running);
        assert_eq!(agents[1].label, "dos");
        assert_eq!(agents[1].state, AgentState::Done);
    }

    #[test]
    fn final_summary_tolerates_garbage() {
        assert!(parse_final("{\"runId\": \"wf_x\", \"workflowProg", "wf_x").is_none());
        let d = parse_final(r#"{"workflowProgress":[{"type":"workflow_phase","index":1,"title":"Solo"},
            {"type":"workflow_agent","label":"x","phaseTitle":"Solo","state":"exploded","tokens":"muchos"}],
            "result":{"status":"purple"}}"#, "wf_y").unwrap();
        assert_eq!(d.workflow_id, "wf_y");
        assert_eq!(d.phases[0].title, "Solo");
        assert_eq!(d.agents[0].state, AgentState::Unknown);
        assert_eq!(d.agents[0].tokens, None);
        assert_eq!(d.result_status, None);
    }

    #[test]
    fn script_phases_best_effort() {
        let src = "export const meta = { name: 'x', phases: [ { title: 'Uno', detail: \"con } llave\" },\n { title: `Dos` } ], }";
        let phases = parse_script_phases(src);
        assert_eq!(phases.len(), 2);
        assert_eq!(phases[0].detail.as_deref(), Some("con } llave"));
        assert_eq!(phases[1].title, "Dos");
        assert!(parse_script_phases("sin fases").is_empty());
    }

    #[test]
    fn agents_json_exposes_blocked_on_permission() {
        // Entrada real de una sesión --bg esperando aprobación de un Write (claude 2.1.281).
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

        // Otro workflow: sin campos conocidos, pero con el JSON crudo.
        let demo = read_run_detail(&fixtures_projects().join(project_slug(SANDBOX)).join(DONE_SESSION)).unwrap();
        let res = demo.result.unwrap();
        assert_eq!((res.pr, res.unmet_acceptance, res.nits), (None, None, None));
        assert!(res.raw.unwrap().contains("\"tema\": \"volcanes\""));

        // PR que no es URL web: se descarta. `null` o ausente: sin resultado.
        let v: Value = serde_json::from_str(r#"{"pr":"javascript:alert(1)","nits":"no-lista"}"#).unwrap();
        let r = parse_result(Some(&v)).unwrap();
        assert_eq!((r.pr, r.nits), (None, None));
        assert_eq!(parse_result(Some(&Value::Null)), None);
        assert_eq!(parse_result(None), None);
        assert_eq!(parse_result(Some(&Value::String("hecho".into()))).unwrap().raw.as_deref(), Some("hecho"));
    }

    #[test]
    fn transcript_from_real_fixture() {
        let dir = fixtures_projects().join(project_slug(SANDBOX)).join(DONE_SESSION);
        let t = read_agent_transcript(&dir, "wf_2450b7a8-254", "a1007a0f03db17270", 200).unwrap().unwrap();
        assert_eq!(t.label.as_deref(), Some("revisar:volcanes-famosos"));
        assert_eq!(t.model.as_deref(), Some("claude-sonnet-5"));
        assert_eq!(t.phase.as_deref(), Some("Revisar"));
        // Sin el marco del harness y sin la indentación.
        let prompt = t.prompt.unwrap();
        assert!(prompt.starts_with("Leé /Users/me/Code/nodal-sandbox/demo-out/04-volcanes-famosos.md"), "{prompt}");
        assert!(!prompt.contains("Workflow harness"));
        // thinking vacío (firmado) se omite: Bash, texto, StructuredOutput.
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

        // Límite: solo los últimos N.
        let t = read_agent_transcript(&dir, "wf_2450b7a8-254", "a1007a0f03db17270", 1).unwrap().unwrap();
        assert_eq!((t.items.len(), t.omitted, t.total_items), (1, 2, 3));
        assert!(matches!(&t.items[0], TranscriptItem::ToolUse { name, .. } if name == "StructuredOutput"));

        // Sin archivo → None; ids con rutas → error.
        assert_eq!(read_agent_transcript(&dir, "wf_2450b7a8-254", "nope", 10).unwrap(), None);
        assert!(read_agent_transcript(&dir, "wf_2450b7a8-254", "../x", 10).is_err());
        assert!(read_agent_transcript(&dir, "..", "a1", 10).is_err());
    }

    #[test]
    fn transcript_tolerates_errors_arrays_and_long_text() {
        let long = "x".repeat(TEXT_MAX + 50);
        let lines = [
            r#"{"type":"user","message":{"role":"user","content":"Tarea sin marco"}}"#.to_string(),
            r#"{"type":"attachment","attachment":{"type":"skill_listing","content":"user assistant"}}"#.to_string(),
            format!(r#"{{"type":"assistant","message":{{"content":[{{"type":"thinking","thinking":"pienso"}},{{"type":"text","text":"{long}"}}]}}}}"#),
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/a/b.rs"}}]}}"#.to_string(),
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":[{"type":"text","text":"File not found"},{"type":"image"}]}]}}"#.to_string(),
            r#"{"type":"user","message":{"content":"¿seguís?"}}"#.to_string(),
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t2","name":"Bash","input":{}}]}}"#.to_string(),
            r#"{"type":"assistant","message":{"content":[{"type":"text","te"#.to_string(),
        ];
        let p = parse_transcript(&lines.join("\n"), 100);
        assert_eq!(p.prompt.as_deref(), Some("Tarea sin marco"));
        assert_eq!(p.total, 5);
        assert!(matches!(&p.items[0], TranscriptItem::Thinking { text, .. } if text == "pienso"));
        assert!(matches!(&p.items[1], TranscriptItem::Text { truncated: true, text } if text.chars().count() == TEXT_MAX + 1));
        match &p.items[2] {
            TranscriptItem::ToolUse { summary, result: Some(r), .. } => {
                assert_eq!(summary.as_deref(), Some("/a/b.rs"));
                assert!(r.is_error);
                assert_eq!(r.text, "File not found\n[image]");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(&p.items[3], TranscriptItem::User { text, .. } if text == "¿seguís?"));
        // Tool sin resultado todavía (agente corriendo) e input vacío.
        assert!(matches!(&p.items[4], TranscriptItem::ToolUse { result: None, input: None, summary: None, .. }));
        // Sin StructuredOutput: la salida final es el último texto.
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
        let tmp = std::env::temp_dir().join(format!("agent-desk-runs-test-{}", std::process::id()));
        fs::create_dir_all(&tmp).unwrap();
        assert_eq!(read_run_detail(&tmp), None);
        let _ = fs::remove_dir_all(&tmp);
    }

    /// Todos los transcripts de workflows de esta máquina: ninguno falla y se miden tiempos.
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

    /// Contra los datos reales de esta máquina: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn real_sessions_on_disk() {
        let projects = claude_config_dir().unwrap().join("projects");
        for session in [DONE_SESSION, CUT_SESSION] {
            let dir = find_session_dir(&projects, SANDBOX, session).expect("sesión real no encontrada");
            let d = read_run_detail(&dir).expect("sin workflow");
            eprintln!("{session}: {d:#?}");
        }
    }
}
