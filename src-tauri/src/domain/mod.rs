//! Modelo de Nodal: proyectos, repos, tareas, runs y fuentes externas.
//! Espejo exacto en `src/domain/types.ts`: cualquier cambio acá va también allá.
//!
//! Convenciones de serialización:
//! - structs en camelCase;
//! - enums "de valor" en snake_case (`in_place`, `in_progress`);
//! - enums con datos llevan el discriminante en `kind`;
//! - fechas en epoch ms (`i64`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Genera `as_str`/`parse` para enums de valor, con los mismos nombres que usa serde.
/// Se usan como representación en SQLite (columnas TEXT).
macro_rules! str_enum {
    ($ty:ident { $($variant:ident => $s:literal),+ $(,)? }) => {
        impl $ty {
            #[cfg(test)]
            #[allow(dead_code)]
            pub const ALL: &'static [$ty] = &[$($ty::$variant),+];
            // Generadas para todos los enums; no todos las usan fuera de los tests.
            #[allow(dead_code)]
            pub fn as_str(self) -> &'static str {
                match self { $($ty::$variant => $s),+ }
            }
            #[allow(dead_code)]
            pub fn parse(s: &str) -> Option<Self> {
                match s { $($s => Some($ty::$variant),)+ _ => None }
            }
        }
    };
}

// ---------- Proyectos y repos ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    /// Prefijo de los ids de tarea (`PAY` → `PAY-1`). Mayúsculas y dígitos, único.
    pub key: String,
    /// Próximo `Task.number` a asignar (empieza en 1).
    pub next_task_number: i64,
    /// Color del proyecto en la UI (hex o `oklch(...)`).
    pub color: String,
    /// Descripción libre (v2).
    #[serde(default)]
    pub description: Option<String>,
    /// Ejecutor por defecto si el repo no define uno. `None` → el global de Settings → Claude.
    pub default_executor: Option<Executor>,
    /// Revisor por defecto si el repo no define uno. `None` → `code-reviewer`.
    pub reviewer: Option<String>,
    pub created_at: i64,
    pub archived_at: Option<i64>,
}

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Repo {
    pub id: String,
    pub project_id: String,
    /// Raíz git canónica. Única en toda la app.
    pub path: String,
    pub name: String,
    /// `model`, `effort` y `permissionMode` van planos en el JSON.
    #[serde(flatten)]
    pub launch: LaunchOptions,
    pub default_executor: Option<Executor>,
    pub default_isolation: Isolation,
    pub default_finish: Finish,
    pub default_review: bool,
    /// Agente revisor. `None` → el del proyecto → `code-reviewer`.
    pub reviewer: Option<String>,
    /// Orden dentro del proyecto.
    pub position: i64,
    pub created_at: i64,
}

// ---------- Ejecutores y opciones ----------

/// De dónde sale la definición de un agente.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSource {
    /// `~/.claude/agents/*.md`
    User,
    /// `<repo>/.claude/agents/*.md`
    Repo,
    /// Un plugin habilitado.
    Plugin,
}
str_enum!(AgentSource { User => "user", Repo => "repo", Plugin => "plugin" });

/// A quién se delega una tarea (su `assignee`) o quién corre un run.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Executor {
    /// `claude --bg --agent <name> "<prompt>"`.
    Agent { name: String, source: AgentSource },
    /// `claude --bg "/<name> <args>"`.
    Workflow { name: String },
    /// Sesión común con la tarea como prompt.
    Claude,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Isolation {
    /// `~/.nodal/worktrees/<repo>/<task-slug>`, rama `nodal/<task-slug>`.
    Worktree,
    /// En la carpeta del repo; la cola no lanza dos a la vez por repo.
    InPlace,
}
str_enum!(Isolation { Worktree => "worktree", InPlace => "in_place" });

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Finish {
    /// Cambios sin commitear.
    Changes,
    /// Commit en la rama, sin push.
    Commit,
    /// Commit, push y PR con `gh`.
    Pr,
}
str_enum!(Finish { Changes => "changes", Commit => "commit", Pr => "pr" });

// ---------- Tareas ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Backlog,
    Todo,
    InProgress,
    InReview,
    Blocked,
    Done,
    Canceled,
}
str_enum!(TaskStatus {
    Backlog => "backlog",
    Todo => "todo",
    InProgress => "in_progress",
    InReview => "in_review",
    Blocked => "blocked",
    Done => "done",
    Canceled => "canceled",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Urgent,
    High,
    Medium,
    Low,
    #[default]
    None,
}
str_enum!(Priority { Urgent => "urgent", High => "high", Medium => "medium", Low => "low", None => "none" });

/// Dónde está el plan. El texto vive en `tasks/<id>/plan.md` (no en la base).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlanRef {
    Text,
    /// Ruta absoluta de un `.md` dentro del repo.
    File { path: String },
}

/// Worktree propio de la tarea (solo con `Isolation::Worktree`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeRef {
    pub path: String,
    pub branch: String,
    /// Ref base contra la que se calcula el diff.
    pub base: String,
}

/// Tipo normalizado de un estado externo, para la heurística de mapeo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtKind {
    Triage,
    Backlog,
    Unstarted,
    Started,
    Completed,
    Canceled,
    /// El proveedor no tipa sus estados (p. ej. secciones de Asana).
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalState {
    pub id: String,
    pub name: String,
    pub kind: ExtKind,
    pub color: Option<String>,
}

/// Vínculo de una tarea importada con su ítem en el proveedor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSource {
    /// `"linear"`, más adelante `"asana"`, `"azure_devops"`...
    pub provider: String,
    /// SourceLink por el que se importó. Para borrar el link, antes se desvinculan sus
    /// tareas (`source = None`): la base no deja borrarlo con tareas apuntándole.
    pub link_id: Option<String>,
    pub external_id: String,
    /// Id legible del proveedor (`ENG-142`).
    pub identifier: String,
    pub url: String,
    pub external_state: Option<ExternalState>,
    pub last_synced_at: Option<i64>,
    pub sync_error: Option<String>,
    /// El estado externo actual no está en el mapeo pull (v2): la tarea conserva su estado
    /// Nodal y la UI muestra "estado externo sin mapear".
    #[serde(default)]
    pub unmapped: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub project_id: String,
    pub repo_id: String,
    /// Número dentro del proyecto: el id visible es `{project.key}-{number}`.
    pub number: i64,
    pub title: String,
    pub status: TaskStatus,
    pub priority: Priority,
    pub labels: Vec<String>,
    /// Orden dentro de la columna (REAL para insertar entre dos sin renumerar).
    pub position: f64,
    pub plan: PlanRef,
    /// En una importada: el plan ya no se regenera desde la descripción externa.
    pub plan_overridden: bool,
    /// Criterios de aceptación contra los que revisa el gate.
    pub acceptance: Vec<String>,
    /// `None` → default del repo → del proyecto → Claude.
    pub assignee: Option<Executor>,
    /// `None` → default del repo.
    pub isolation: Option<Isolation>,
    pub finish: Option<Finish>,
    pub review: Option<bool>,
    pub worktree: Option<WorktreeRef>,
    pub source: Option<TaskSource>,
    pub created_at: i64,
    pub updated_at: i64,
    /// Cuándo pasó a Done o Canceled.
    pub closed_at: Option<i64>,
}

/// `{KEY}-{number}`, p. ej. `PAY-1`.
pub fn task_key(project_key: &str, number: i64) -> String {
    format!("{project_key}-{number}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    Related,
    /// `task_id` bloquea a `other_id`.
    Blocks,
}
str_enum!(RelationKind { Related => "related", Blocks => "blocks" });

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRelation {
    pub task_id: String,
    pub other_id: String,
    pub kind: RelationKind,
}

// ---------- Runs ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunKind {
    Work,
    Review,
}
str_enum!(RunKind { Work => "work", Review => "review" });

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Queued,
    Launching,
    Launched,
    Finished,
    Failed,
    Canceled,
}
str_enum!(RunStatus {
    Queued => "queued",
    Launching => "launching",
    Launched => "launched",
    Finished => "finished",
    Failed => "failed",
    Canceled => "canceled",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    Green,
    Yellow,
    Red,
    Stopped,
    Unknown,
}
str_enum!(RunOutcome { Green => "green", Yellow => "yellow", Red => "red", Stopped => "stopped", Unknown => "unknown" });

/// Veredicto del revisor (o el resultado estructurado de un workflow que revisa).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Verdict {
    pub pass: bool,
    pub unmet: Vec<String>,
    pub nits: Vec<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: String,
    /// `None` en runs sin tarea (lanzados a mano o migrados).
    pub task_id: Option<String>,
    pub repo_id: Option<String>,
    pub cwd: String,
    pub executor: Executor,
    pub kind: RunKind,
    /// Paso anterior de la cadena (el run revisado, o el previo a un traspaso).
    pub parent_run_id: Option<String>,
    pub prompt: String,
    /// "Extra instructions for this run", ya incluidas en `prompt`.
    pub extra_instructions: Option<String>,
    pub options: LaunchOptions,
    pub finish: Finish,
    /// Resuelta al encolar (tarea → repo). `None` en workflows, que manejan su worktree.
    pub isolation: Option<Isolation>,
    /// Resuelto al encolar: al terminar este run de trabajo, se encola el revisor.
    pub review: bool,
    pub verdict: Option<Verdict>,
    pub status: RunStatus,
    /// Orden en la cola global (menor sale antes). REAL para reordenar sin renumerar.
    pub queue_position: f64,
    /// Id corto de `claude --bg`.
    pub claude_run_id: Option<String>,
    pub session_id: Option<String>,
    pub queued_at: i64,
    pub launched_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub outcome: Option<RunOutcome>,
    /// `summary` del bloque JSON final del ejecutor (se pasa al siguiente paso).
    pub summary: Option<String>,
    pub pr_url: Option<String>,
    pub branch: Option<String>,
    pub error: Option<String>,
    /// Etiqueta del run importado de una versión anterior (issue o tarea vieja).
    pub legacy_label: Option<String>,
    /// Tokens del transcript (input + output + cache), sumados al cerrar (v2). `None` en
    /// workflows o si no se pudo leer.
    #[serde(default)]
    pub tokens: Option<i64>,
}

// ---------- Fuentes externas ----------

/// Scope del proveedor que se vincula (team o proyecto de Linear, proyecto de Asana...).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeRef {
    /// Depende del proveedor: `"team"`, `"project"`...
    pub kind: String,
    pub id: String,
    pub name: String,
}

/// Al importar, una tarea con este label va a este repo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoRule {
    pub label: String,
    pub repo_id: String,
}

/// Mapeo de estados en las dos direcciones. Lo propone Nodal; lo confirma el usuario.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateMap {
    /// `external_state.id` → estado Nodal. Un id ausente está "sin mapear".
    pub pull: BTreeMap<String, TaskStatus>,
    /// Estado Nodal → `external_state.id`; `null` = "No sincronizar" (solo comentario).
    pub push: BTreeMap<TaskStatus, Option<String>>,
    /// `None` = mapeo pendiente: se puede importar pero no se hace push.
    pub confirmed_at: Option<i64>,
    /// Foto de los estados del proveedor al confirmar, para detectar altas y bajas.
    pub known_states: Vec<ExternalState>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceLink {
    pub id: String,
    pub project_id: String,
    pub provider: String,
    pub scope: ScopeRef,
    pub default_repo_id: Option<String>,
    pub repo_rules: Vec<RepoRule>,
    pub state_map: StateMap,
    pub auto_import: bool,
    pub created_at: i64,
    /// Última pasada del sync sobre este link (v2).
    #[serde(default)]
    pub last_synced_at: Option<i64>,
    /// Error de la última pasada (`None` si fue bien).
    #[serde(default)]
    pub last_sync_error: Option<String>,
    /// Altas/bajas de estados sin revisar. `None` = nada pendiente.
    #[serde(default)]
    pub pending_state_changes: Option<StateChanges>,
}

/// Estados del proveedor que cambiaron contra `StateMap.known_states` (v2). Lo deja el
/// sync; se limpia al guardar el mapeo.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateChanges {
    pub added: Vec<ExternalState>,
    pub removed: Vec<ExternalState>,
}

impl StateChanges {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

/// Qué hay que escribir en el proveedor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OutboxPayload {
    #[serde(rename_all = "camelCase")]
    SetState { state_id: String },
    Comment { body: String },
}

impl OutboxPayload {
    pub fn kind(&self) -> &'static str {
        match self {
            OutboxPayload::SetState { .. } => "set_state",
            OutboxPayload::Comment { .. } => "comment",
        }
    }
}

/// Escritura pendiente en el proveedor (push de estado o comentario), con backoff.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboxItem {
    /// Autoincremental; 0 antes de insertar.
    pub id: i64,
    pub task_id: String,
    pub provider: String,
    pub payload: OutboxPayload,
    pub attempts: i64,
    pub next_attempt_at: i64,
    pub last_error: Option<String>,
    pub created_at: i64,
}

// ---------- Settings ----------

pub const DEFAULT_CONCURRENCY: u32 = 3;
pub const MAX_CONCURRENCY: u32 = 16;
pub const DEFAULT_REVIEWER: &str = "code-reviewer";

/// Ajustes globales. En la base es una fila por campo (`settings.key` = nombre camelCase).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Runs simultáneos en la cola global.
    pub concurrency: u32,
    /// Comando del editor para "Open in editor" (`code`, `cursor`...). `None` → el del sistema.
    pub editor: Option<String>,
    /// Revisor global si ni el repo ni el proyecto definen uno.
    pub reviewer: String,
    /// Último fallback del ejecutor: repo → proyecto → este → Claude.
    pub default_executor: Option<Executor>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { concurrency: DEFAULT_CONCURRENCY, editor: None, reviewer: DEFAULT_REVIEWER.into(), default_executor: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn enums_serialize_snake_case_and_match_as_str() {
        for s in TaskStatus::ALL {
            assert_eq!(serde_json::to_value(s).unwrap(), json!(s.as_str()));
            assert_eq!(TaskStatus::parse(s.as_str()), Some(*s));
        }
        for s in Isolation::ALL {
            assert_eq!(serde_json::to_value(s).unwrap(), json!(s.as_str()));
        }
        for s in RunStatus::ALL {
            assert_eq!(serde_json::to_value(s).unwrap(), json!(s.as_str()));
        }
        for s in RunOutcome::ALL {
            assert_eq!(serde_json::to_value(s).unwrap(), json!(s.as_str()));
        }
        for s in Priority::ALL {
            assert_eq!(serde_json::to_value(s).unwrap(), json!(s.as_str()));
        }
        assert_eq!(TaskStatus::parse("nope"), None);
    }

    #[test]
    fn executor_shape() {
        let a = Executor::Agent { name: "frontend-developer".into(), source: AgentSource::User };
        assert_eq!(
            serde_json::to_value(&a).unwrap(),
            json!({"kind": "agent", "name": "frontend-developer", "source": "user"})
        );
        assert_eq!(serde_json::to_value(Executor::Claude).unwrap(), json!({"kind": "claude"}));
        assert_eq!(
            serde_json::to_value(Executor::Workflow { name: "plan-task".into() }).unwrap(),
            json!({"kind": "workflow", "name": "plan-task"})
        );
    }

    #[test]
    fn outbox_payload_shape() {
        let p = OutboxPayload::SetState { state_id: "s3".into() };
        assert_eq!(serde_json::to_value(&p).unwrap(), json!({"kind": "set_state", "stateId": "s3"}));
        assert_eq!(p.kind(), "set_state");
        let c = OutboxPayload::Comment { body: "hola".into() };
        assert_eq!(serde_json::to_value(&c).unwrap(), json!({"kind": "comment", "body": "hola"}));
        assert_eq!(c.kind(), "comment");
    }

    #[test]
    fn repo_flattens_launch_options() {
        let r = Repo {
            id: "r".into(),
            project_id: "p".into(),
            path: "/x".into(),
            name: "x".into(),
            launch: LaunchOptions { model: Some("opus".into()), ..Default::default() },
            default_executor: None,
            default_isolation: Isolation::InPlace,
            default_finish: Finish::Pr,
            default_review: true,
            reviewer: None,
            position: 0,
            created_at: 1,
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["model"], "opus");
        assert_eq!(v["defaultIsolation"], "in_place");
        assert!(v.get("effort").is_none());
        assert_eq!(serde_json::from_value::<Repo>(v).unwrap(), r);
    }

    #[test]
    fn state_map_keys_are_status_strings() {
        let mut m = StateMap::default();
        m.pull.insert("s1".into(), TaskStatus::InReview);
        m.push.insert(TaskStatus::Blocked, None);
        m.push.insert(TaskStatus::InProgress, Some("s2".into()));
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["pull"]["s1"], "in_review");
        assert_eq!(v["push"]["blocked"], serde_json::Value::Null);
        assert_eq!(v["push"]["in_progress"], "s2");
        assert_eq!(serde_json::from_value::<StateMap>(v).unwrap(), m);
    }

    #[test]
    fn settings_defaults_fill_missing_fields() {
        let s: Settings = serde_json::from_value(json!({"editor": "code"})).unwrap();
        assert_eq!(s.concurrency, DEFAULT_CONCURRENCY);
        assert_eq!(s.reviewer, DEFAULT_REVIEWER);
        assert_eq!(s.editor.as_deref(), Some("code"));
        assert_eq!(task_key("PAY", 1), "PAY-1");
    }
}
