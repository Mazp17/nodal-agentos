//! Tareas locales (sin Linear) y sus runs, persistidas en `<app_data_dir>`:
//! - `tasks.json`: las tareas,
//! - `tasks/<id>/plan.md`: el plan de las tareas con plan en texto,
//! - `task-runs.json`: asociación tarea ↔ run y cola de lanzamientos.
//!
//! Acá solo hay datos y lógica pura (testeable); los efectos viven en `mod.rs`.
//! La cola es análoga a la de `issue_runs::store` (ver el reporte de unificación en
//! `mod.rs`): mismos estados, misma gracia de lanzamiento, mismo cálculo de slots.

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::runs::types::RunSummary;

pub const TASKS_FILE: &str = "tasks.json";
pub const RUNS_FILE: &str = "task-runs.json";
pub const PLANS_DIR: &str = "tasks";
pub const PLAN_FILE: &str = "plan.md";
/// Historial máximo de runs guardado (los en cola nunca se descartan).
const MAX_HISTORY: usize = 300;
/// Espejo de `issue_runs::store::LAUNCH_GRACE_MS`.
pub const LAUNCH_GRACE_MS: i64 = 90_000;
/// Espejo de `issue_runs::store::SESSION_LOOKUP_MS`.
pub const SESSION_LOOKUP_MS: i64 = 30 * 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Todo,
    Done,
}

/// Dónde está el plan. El texto vive en `tasks/<id>/plan.md` (no se duplica acá).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum PlanRef {
    Text,
    /// Ruta absoluta (canonicalizada al crear) de un `.md` dentro del repo.
    File { path: String },
}

/// Lo que manda el frontend al crear/editar.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum PlanInput {
    Text { text: String },
    /// Absoluta, o relativa al repo.
    File { path: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub repo_path: String,
    pub title: String,
    pub plan: PlanRef,
    pub status: TaskStatus,
    /// Epoch ms.
    pub created_at: i64,
    #[serde(default)]
    pub done_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskRunStatus {
    Queued,
    Launching,
    Launched,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRun {
    pub task_id: String,
    /// El prompt completo (`/plan-task {...}`), armado al encolar.
    pub prompt: String,
    /// "pr" | "branch".
    pub finish: String,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    pub cwd: String,
    pub queued_at: i64,
    #[serde(default)]
    pub launched_at: Option<i64>,
    pub status: TaskRunStatus,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct TasksData {
    #[serde(default)]
    pub tasks: Vec<Task>,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunsData {
    #[serde(default)]
    pub runs: Vec<TaskRun>,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Ids
// ---------------------------------------------------------------------------

static SEQ: AtomicU32 = AtomicU32::new(0);

/// Id ordenable por tiempo, estilo ULID sin dependencias: `t` + ms en base32 (10) +
/// secuencia del proceso (3) + 5 caracteres aleatorios (hash con semilla de `RandomState`).
/// Solo `[0-9a-z]`: termina en rutas (`tasks/<id>/`).
pub fn new_id(now: i64) -> String {
    use std::hash::{BuildHasher, Hasher};
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_i64(now);
    h.write_u32(seq);
    h.write_u32(std::process::id());
    let rnd = h.finish();
    format!("t{}{}{}", base32(now as u64, 10), base32(seq as u64, 3), base32(rnd, 5))
}

fn base32(mut n: u64, width: usize) -> String {
    const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";
    let mut out = vec![b'0'; width];
    for slot in out.iter_mut().rev() {
        *slot = ALPHABET[(n % 32) as usize];
        n /= 32;
    }
    String::from_utf8(out).unwrap_or_default()
}

/// Un id de tarea viene del frontend y termina en una ruta.
pub fn is_valid_task_id(id: &str) -> bool {
    (2..=40).contains(&id.len()) && id.starts_with('t') && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

// ---------------------------------------------------------------------------
// Persistencia
// ---------------------------------------------------------------------------

fn load_json<T: DeserializeOwned + Default>(path: &Path) -> Result<T, String> {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| format!("{} is corrupt: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(format!("Couldn't read {}: {e}", path.display())),
    }
}

/// Escritura atómica: temporal + rename.
pub fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&tmp, contents).map_err(|e| format!("Couldn't write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("Couldn't save {}: {e}", path.display()))
}

fn save_json<T: Serialize>(path: &Path, data: &T) -> Result<(), String> {
    let json = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
    write_atomic(path, json.as_bytes())
}

pub fn load_tasks(path: &Path) -> Result<TasksData, String> {
    load_json(path)
}

pub fn save_tasks(path: &Path, data: &TasksData) -> Result<(), String> {
    save_json(path, data)
}

/// Un `launching` que quedó de una sesión anterior no se sabe si llegó a lanzarse:
/// pasa a `failed` con explicación (igual que en `issue_runs`).
pub fn load_runs(path: &Path) -> Result<RunsData, String> {
    let mut data: RunsData = load_json(path)?;
    for r in data.runs.iter_mut().filter(|r| r.status == TaskRunStatus::Launching) {
        r.status = TaskRunStatus::Failed;
        r.error = Some("The app closed while the run was launching; check the runs list before retrying.".into());
    }
    Ok(data)
}

pub fn save_runs(path: &Path, data: &RunsData) -> Result<(), String> {
    save_json(path, data)
}

// ---------------------------------------------------------------------------
// Cola (misma lógica que issue_runs::store)
// ---------------------------------------------------------------------------

pub fn prune(data: &mut RunsData) {
    if data.runs.len() <= MAX_HISTORY {
        return;
    }
    data.runs.sort_by_key(|r| std::cmp::Reverse(r.queued_at));
    let mut kept = 0;
    data.runs.retain(|r| {
        if matches!(r.status, TaskRunStatus::Queued | TaskRunStatus::Launching) {
            return true;
        }
        kept += 1;
        kept <= MAX_HISTORY
    });
}

/// ¿El run sigue activo? En cola/lanzándose, o lanzado y `working` (o recién lanzado y
/// todavía sin aparecer en `claude agents`).
pub fn is_active(r: &TaskRun, live: Option<&[RunSummary]>, now: i64) -> bool {
    match r.status {
        TaskRunStatus::Queued | TaskRunStatus::Launching => true,
        TaskRunStatus::Failed => false,
        TaskRunStatus::Launched => {
            let Some(live) = live else { return false };
            match live.iter().find(|s| Some(&s.id) == r.run_id.as_ref()) {
                Some(s) => matches!(s.state.as_deref(), Some("working") | Some("blocked")),
                None => r.launched_at.is_some_and(|t| now - t < LAUNCH_GRACE_MS),
            }
        }
    }
}

/// Slots ocupados: toda sesión en background `working` (de la app o no, incluidas las
/// de `issue_runs`), más las nuestras que se están lanzando o que aún no figuran.
pub fn occupied_slots(runs: &[TaskRun], live: &[RunSummary], now: i64) -> usize {
    let working = live.iter().filter(|s| s.state.as_deref() == Some("working")).count();
    let pending = runs
        .iter()
        .filter(|r| match r.status {
            TaskRunStatus::Launching => true,
            TaskRunStatus::Launched => {
                !live.iter().any(|s| Some(&s.id) == r.run_id.as_ref())
                    && r.launched_at.is_some_and(|t| now - t < LAUNCH_GRACE_MS)
            }
            _ => false,
        })
        .count();
    working + pending
}

/// Índices de los runs en cola a lanzar ahora (FIFO por `queued_at`) según los slots libres.
pub fn next_to_launch(runs: &[TaskRun], live: &[RunSummary], concurrency: u32, now: i64) -> Vec<usize> {
    let free = (concurrency as usize).saturating_sub(occupied_slots(runs, live, now));
    let mut queued: Vec<usize> = (0..runs.len()).filter(|&i| runs[i].status == TaskRunStatus::Queued).collect();
    queued.sort_by_key(|&i| (runs[i].queued_at, i));
    queued.truncate(free);
    queued
}

pub fn fill_session_ids(runs: &mut [TaskRun], live: &[RunSummary]) -> bool {
    let mut changed = false;
    for r in runs.iter_mut().filter(|r| r.session_id.is_none()) {
        if let Some(s) = live.iter().find(|s| Some(&s.id) == r.run_id.as_ref()) {
            r.session_id = Some(s.session_id.clone());
            changed = true;
        }
    }
    changed
}

pub fn needs_tick(runs: &[TaskRun], now: i64) -> bool {
    runs.iter().any(|r| {
        r.status == TaskRunStatus::Queued
            || (r.status == TaskRunStatus::Launched
                && r.session_id.is_none()
                && r.launched_at.is_some_and(|t| now - t < SESSION_LOOKUP_MS))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(task: &str, status: TaskRunStatus, queued_at: i64) -> TaskRun {
        TaskRun {
            task_id: task.into(),
            prompt: "/plan-task {}".into(),
            finish: "pr".into(),
            run_id: None,
            session_id: None,
            cwd: "/repo".into(),
            queued_at,
            launched_at: None,
            status,
            error: None,
        }
    }

    fn launched(task: &str, run_id: &str, at: i64) -> TaskRun {
        TaskRun { run_id: Some(run_id.into()), launched_at: Some(at), ..run(task, TaskRunStatus::Launched, at - 1) }
    }

    fn live(id: &str, state: &str) -> RunSummary {
        RunSummary {
            id: id.into(),
            session_id: format!("sess-{id}"),
            cwd: Some("/repo".into()),
            name: None,
            started_at: None,
            pid: None,
            status: None,
            state: Some(state.into()),
        }
    }

    const NOW: i64 = 10_000_000;

    #[test]
    fn ids_are_unique_sortable_and_path_safe() {
        let a = new_id(1_700_000_000_000);
        let b = new_id(1_700_000_000_000);
        let c = new_id(1_700_000_000_001);
        assert_ne!(a, b);
        assert!(a < c && b < c, "{a} {b} {c}");
        assert_eq!(a.len(), 19);
        assert!(is_valid_task_id(&a), "{a}");
        assert!(!is_valid_task_id("../etc"));
        assert!(!is_valid_task_id("t/../x"));
        assert!(!is_valid_task_id("TABC"));
        assert!(!is_valid_task_id(""));
    }

    #[test]
    fn queue_is_fifo_and_counts_foreign_working_sessions() {
        let runs = vec![
            run("c", TaskRunStatus::Queued, 30),
            run("a", TaskRunStatus::Queued, 10),
            run("b", TaskRunStatus::Queued, 20),
            run("x", TaskRunStatus::Failed, 1),
        ];
        let lv = vec![live("aa11", "working"), live("bb22", "done")];
        assert_eq!(next_to_launch(&runs, &lv, 3, NOW), vec![1, 2]);
        assert_eq!(next_to_launch(&runs, &lv, 1, NOW), Vec::<usize>::new());
        assert_eq!(next_to_launch(&runs, &[], 10, NOW), vec![1, 2, 0]);
    }

    #[test]
    fn launching_and_fresh_launches_occupy_slots() {
        let runs = vec![
            run("q", TaskRunStatus::Queued, 5),
            run("l", TaskRunStatus::Launching, 1),
            launched("f", "ffff0001", NOW - 1_000),
            launched("o", "ffff0002", NOW - LAUNCH_GRACE_MS - 1),
        ];
        assert_eq!(occupied_slots(&runs, &[], NOW), 2);
        assert_eq!(next_to_launch(&runs, &[], 3, NOW), vec![0]);
        let lv = vec![live("ffff0001", "working")];
        assert_eq!(occupied_slots(&runs, &lv, NOW), 2);
    }

    #[test]
    fn active_and_session_fill() {
        let lv = vec![live("aaaa", "working"), live("bbbb", "done"), live("cccc", "blocked")];
        assert!(is_active(&run("q", TaskRunStatus::Queued, 1), None, NOW));
        assert!(is_active(&launched("a", "aaaa", 1), Some(&lv), NOW));
        assert!(!is_active(&launched("b", "bbbb", NOW), Some(&lv), NOW));
        assert!(is_active(&launched("c", "cccc", 1), Some(&lv), NOW));
        assert!(is_active(&launched("d", "dddd", NOW - 10), Some(&lv), NOW));

        let mut runs = vec![launched("a", "aaaa", NOW)];
        assert!(needs_tick(&runs, NOW));
        assert!(fill_session_ids(&mut runs, &lv));
        assert_eq!(runs[0].session_id.as_deref(), Some("sess-aaaa"));
        assert!(!needs_tick(&runs, NOW));
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("agent-desk-tasks-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn persistence_roundtrip_and_recovery() {
        let dir = temp_dir("store");
        let tp = dir.join("nested").join(TASKS_FILE);
        let rp = dir.join("nested").join(RUNS_FILE);
        assert_eq!(load_tasks(&tp).unwrap(), TasksData::default());
        assert_eq!(load_runs(&rp).unwrap(), RunsData::default());

        let tasks = TasksData {
            tasks: vec![
                Task {
                    id: "t1".into(),
                    repo_path: "/repo".into(),
                    title: "Do it".into(),
                    plan: PlanRef::Text,
                    status: TaskStatus::Todo,
                    created_at: 5,
                    done_at: None,
                },
                Task {
                    id: "t2".into(),
                    repo_path: "/repo".into(),
                    title: "Other".into(),
                    plan: PlanRef::File { path: "/repo/PLAN.md".into() },
                    status: TaskStatus::Done,
                    created_at: 6,
                    done_at: Some(9),
                },
            ],
        };
        save_tasks(&tp, &tasks).unwrap();
        assert_eq!(load_tasks(&tp).unwrap(), tasks);
        let raw = std::fs::read_to_string(&tp).unwrap();
        assert!(raw.contains("\"repoPath\"") && raw.contains("\"kind\": \"file\"") && raw.contains("\"todo\""), "{raw}");

        let runs = RunsData { runs: vec![run("t1", TaskRunStatus::Queued, 1), run("t2", TaskRunStatus::Launching, 2)] };
        save_runs(&rp, &runs).unwrap();
        let back = load_runs(&rp).unwrap();
        assert_eq!(back.runs[0], runs.runs[0]);
        assert_eq!(back.runs[1].status, TaskRunStatus::Failed);
        assert!(back.runs[1].error.as_deref().unwrap().contains("closed"));

        std::fs::write(&tp, "{nope").unwrap();
        assert!(load_tasks(&tp).unwrap_err().contains("corrupt"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn plan_input_json_shape() {
        let t: PlanInput = serde_json::from_str(r##"{"kind":"text","text":"# Plan"}"##).unwrap();
        assert_eq!(t, PlanInput::Text { text: "# Plan".into() });
        let f: PlanInput = serde_json::from_str(r#"{"kind":"file","path":"docs/p.md"}"#).unwrap();
        assert_eq!(f, PlanInput::File { path: "docs/p.md".into() });
        assert!(serde_json::from_str::<PlanInput>(r#"{"kind":"url","path":"x"}"#).is_err());
    }

    #[test]
    fn prune_keeps_queued() {
        let mut data = RunsData::default();
        data.runs.push(run("q", TaskRunStatus::Queued, 0));
        for i in 1..=(MAX_HISTORY as i64 + 20) {
            data.runs.push(run(&i.to_string(), TaskRunStatus::Failed, i));
        }
        prune(&mut data);
        assert!(data.runs.len() <= MAX_HISTORY + 1);
        assert!(data.runs.iter().any(|r| r.status == TaskRunStatus::Queued));
    }
}
