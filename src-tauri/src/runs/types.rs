//! Tipos que el módulo `runs` expone al frontend. Espejo en `src/features/runs/types.ts`.
//! Son nuestros, no de Claude Code: el parseo del formato interno vive en `claude_fs.rs`.

use serde::Serialize;

/// Lo que devuelve `launch_run`: el id corto que imprime `claude --bg`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRef {
    pub id: String,
    pub cwd: String,
}

/// Una sesión en background según `claude agents --json --all`.
/// Las sesiones detenidas no traen `pid` ni `status`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub id: String,
    pub session_id: String,
    pub cwd: Option<String>,
    pub name: Option<String>,
    /// Epoch en ms.
    pub started_at: Option<i64>,
    pub pid: Option<u32>,
    /// "busy" | "idle" (solo sesiones vivas).
    pub status: Option<String>,
    /// "working" | "done" | "stopped".
    pub state: Option<String>,
}

/// De dónde salió el detalle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DetailSource {
    /// `workflows/wf_*.json`: el workflow terminó (o al menos escribió su resumen).
    Final,
    /// Reconstruido desde `journal.jsonl`: en curso, o cortado sin resumen.
    Live,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhaseInfo {
    pub title: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentState {
    Queued,
    Running,
    Done,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub agent_id: Option<String>,
    pub label: String,
    pub phase: Option<String>,
    pub model: Option<String>,
    pub state: AgentState,
    pub tokens: Option<u64>,
    pub tool_calls: Option<u64>,
    pub duration_ms: Option<u64>,
    pub last_tool_name: Option<String>,
    pub last_tool_summary: Option<String>,
    pub result_preview: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDetail {
    /// "wf_2450b7a8-254".
    pub workflow_id: String,
    pub workflow_name: Option<String>,
    pub source: DetailSource,
    /// Estado del workflow según Claude Code ("completed", ...). `None` en modo `live`:
    /// el journal no distingue "en curso" de "cortado"; cruzarlo con `RunSummary.state`.
    pub status: Option<String>,
    pub phases: Vec<PhaseInfo>,
    pub current_phase: Option<String>,
    /// 1-based dentro de `phases`.
    pub current_phase_index: Option<u32>,
    pub agents: Vec<AgentInfo>,
    pub agent_count: u32,
    pub total_tokens: Option<u64>,
    pub total_tool_calls: Option<u64>,
    pub duration_ms: Option<u64>,
    /// `result.status` del workflow si es "green" | "yellow" | "red".
    pub result_status: Option<String>,
    /// Cuántos workflows tiene la sesión (se devuelve solo el más reciente).
    pub workflow_count: u32,
}
