//! Tipos que el módulo `runs` expone al frontend. Espejo en `src/features/runs/types.ts`.
//! Son nuestros, no de Claude Code: el parseo del formato interno vive en `claude_fs.rs`.

use serde::{Deserialize, Serialize};

/// Flags opcionales de `claude --bg` que se configuran por repo.
/// Valores permitidos: ver `runs::options` (verificados contra `claude --help` 2.1.281).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
}

impl LaunchOptions {
    pub fn is_empty(&self) -> bool {
        self.model.is_none() && self.effort.is_none() && self.permission_mode.is_none()
    }
}

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
    /// "busy" | "idle" | "waiting" (solo sesiones vivas).
    pub status: Option<String>,
    /// "working" | "blocked" | "done" | "failed" | "stopped".
    pub state: Option<String>,
    /// Con `status == "waiting"`: "permission prompt", "input needed", "sandbox request",
    /// "worker request", "dialog open" (valores de la doc de agent view; verificado el primero).
    pub waiting_for: Option<String>,
}

impl RunSummary {
    /// La sesión sigue en curso: trabajando, o bloqueada esperando al usuario (permiso,
    /// input). En ambos casos ocupa un slot y la issue no se puede relanzar.
    pub fn is_in_progress(&self) -> bool {
        matches!(self.state.as_deref(), Some("working" | "blocked"))
    }
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
    /// Campos conocidos del `result` del workflow (solo en modo `final`).
    pub result: Option<RunResult>,
    /// Cuántos workflows tiene la sesión (se devuelve solo el más reciente).
    pub workflow_count: u32,
}

/// Campos conocidos del `result` que devuelve un workflow (p. ej. `linear-issue`).
/// `result` es libre: cada campo es opcional y se ignora si no tiene el tipo esperado.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RunResult {
    pub issue: Option<String>,
    /// URL del PR (solo http/https).
    pub pr: Option<String>,
    pub branch: Option<String>,
    pub workdir: Option<String>,
    /// Dónde se cortó (linear-issue en Blocked).
    #[serde(rename = "where")]
    pub where_: Option<String>,
    pub unmet_acceptance: Option<Vec<String>>,
    pub nits: Option<Vec<String>>,
    /// `result` completo como JSON indentado (recortado), para mostrar lo que no se conoce.
    pub raw: Option<String>,
}

/// Una entrada de la conversación de un subagente.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TranscriptItem {
    /// Texto del asistente.
    #[serde(rename_all = "camelCase")]
    Text { text: String, truncated: bool },
    /// Razonamiento del asistente (solo si viene en claro; casi siempre viene vacío).
    #[serde(rename_all = "camelCase")]
    Thinking { text: String, truncated: bool },
    /// Mensaje del lado del usuario que no es un resultado de tool.
    #[serde(rename_all = "camelCase")]
    User { text: String, truncated: bool },
    #[serde(rename_all = "camelCase")]
    ToolUse {
        id: Option<String>,
        name: String,
        /// Una línea: el argumento principal (comando, ruta, patrón...).
        summary: Option<String>,
        /// Input completo como JSON indentado, recortado.
        input: Option<String>,
        result: Option<ToolResultInfo>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultInfo {
    pub text: String,
    pub is_error: bool,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transcript {
    pub agent_id: String,
    pub label: Option<String>,
    pub model: Option<String>,
    pub phase: Option<String>,
    /// Tarea que recibió el agente (texto calculado por el workflow).
    pub prompt: Option<String>,
    pub items: Vec<TranscriptItem>,
    /// Items totales en lo leído; los primeros `omitted` no se devuelven.
    pub total_items: u32,
    pub omitted: u32,
    /// Salida final: input de `StructuredOutput` o último texto del asistente.
    pub final_output: Option<String>,
    /// El archivo era demasiado grande y solo se leyó su principio y su cola.
    pub partial: bool,
    pub bytes: u64,
}
