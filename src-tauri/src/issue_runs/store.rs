//! Asociación issue ↔ run y cola de lanzamientos, persistidas en
//! `<app_data_dir>/issue-runs.json`. Acá solo hay datos y lógica pura (testeable);
//! los efectos (lanzar, `claude agents`) viven en `mod.rs`.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::runs::types::{LaunchOptions, RunSummary};

pub const FILE_NAME: &str = "issue-runs.json";
/// Historial máximo guardado (los en cola nunca se descartan).
const MAX_HISTORY: usize = 300;
/// Un run recién lanzado tarda en aparecer en `claude agents`: mientras tanto ocupa un slot.
pub const LAUNCH_GRACE_MS: i64 = 90_000;
/// Pasado este tiempo sin aparecer en `claude agents`, se deja de buscar su sessionId.
pub const SESSION_LOOKUP_MS: i64 = 30 * 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueRunStatus {
    /// Esperando un slot libre.
    Queued,
    /// `claude --bg` en curso.
    Launching,
    /// Lanzado: el estado real (working/done/stopped) sale de `claude agents`.
    Launched,
    /// No se pudo lanzar; ver `error`.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueRun {
    pub issue_id: String,
    pub identifier: String,
    pub workflow: String,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    pub cwd: String,
    /// Epoch ms.
    pub queued_at: i64,
    #[serde(default)]
    pub launched_at: Option<i64>,
    pub status: IssueRunStatus,
    #[serde(default)]
    pub error: Option<String>,
    /// Model/effort/permission mode del repo, fijados al encolar.
    #[serde(default, skip_serializing_if = "LaunchOptions::is_empty")]
    pub options: LaunchOptions,
}

impl IssueRun {
    pub fn prompt(&self) -> String {
        format!("/{} {}", self.workflow, self.identifier)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoreData {
    #[serde(default)]
    pub runs: Vec<IssueRun>,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Carga el archivo. Si no existe, vacío. Un `launching` que quedó de una sesión
/// anterior no se sabe si llegó a lanzarse: pasa a `failed` con explicación.
pub fn load_from(path: &Path) -> Result<StoreData, String> {
    let mut data: StoreData = match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| format!("{} is corrupt: {e}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => StoreData::default(),
        Err(e) => return Err(format!("Couldn't read {}: {e}", path.display())),
    };
    for r in data.runs.iter_mut().filter(|r| r.status == IssueRunStatus::Launching) {
        r.status = IssueRunStatus::Failed;
        r.error = Some(
            "The app closed while the run was launching; check the runs list before retrying.".into(),
        );
    }
    Ok(data)
}

/// Escritura atómica: temporal + rename.
pub fn save_to(path: &Path, data: &StoreData) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    }
    let json = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&tmp, json).map_err(|e| format!("Couldn't write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("Couldn't save {}: {e}", path.display()))
}

/// Recorta el historial viejo (terminados), sin tocar los en cola ni los que se lanzan.
pub fn prune(data: &mut StoreData) {
    if data.runs.len() <= MAX_HISTORY {
        return;
    }
    data.runs.sort_by_key(|r| std::cmp::Reverse(r.queued_at));
    let mut kept = 0;
    data.runs.retain(|r| {
        let active = matches!(r.status, IssueRunStatus::Queued | IssueRunStatus::Launching);
        kept += 1;
        active || kept <= MAX_HISTORY
    });
}

/// ¿El run de esta issue sigue activo? En cola/lanzándose, o lanzado y `working`
/// (o recién lanzado y todavía sin aparecer en `claude agents`).
pub fn is_active(r: &IssueRun, live: Option<&[RunSummary]>, now: i64) -> bool {
    match r.status {
        IssueRunStatus::Queued | IssueRunStatus::Launching => true,
        IssueRunStatus::Failed => false,
        IssueRunStatus::Launched => {
            let Some(live) = live else { return false };
            match live.iter().find(|s| Some(&s.id) == r.run_id.as_ref()) {
                Some(s) => s.is_in_progress(),
                None => r.launched_at.is_some_and(|t| now - t < LAUNCH_GRACE_MS),
            }
        }
    }
}

/// Slots ocupados: toda sesión en background `working` (sea nuestra o no), más los
/// nuestros que se están lanzando o que se lanzaron hace poco y aún no figuran.
pub fn occupied_slots(runs: &[IssueRun], live: &[RunSummary], now: i64) -> usize {
    let working = live.iter().filter(|s| s.is_in_progress()).count();
    let pending = runs
        .iter()
        .filter(|r| match r.status {
            IssueRunStatus::Launching => true,
            IssueRunStatus::Launched => {
                !live.iter().any(|s| Some(&s.id) == r.run_id.as_ref())
                    && r.launched_at.is_some_and(|t| now - t < LAUNCH_GRACE_MS)
            }
            _ => false,
        })
        .count();
    working + pending
}

/// Índices de los runs en cola a lanzar ahora (FIFO por `queued_at`) según los slots libres.
pub fn next_to_launch(runs: &[IssueRun], live: &[RunSummary], concurrency: u32, now: i64) -> Vec<usize> {
    let free = (concurrency as usize).saturating_sub(occupied_slots(runs, live, now));
    let mut queued: Vec<usize> = (0..runs.len()).filter(|&i| runs[i].status == IssueRunStatus::Queued).collect();
    queued.sort_by_key(|&i| (runs[i].queued_at, i));
    queued.truncate(free);
    queued
}

/// Completa `session_id` cruzando `run_id` con `claude agents`. Devuelve si cambió algo.
pub fn fill_session_ids(runs: &mut [IssueRun], live: &[RunSummary]) -> bool {
    let mut changed = false;
    for r in runs.iter_mut().filter(|r| r.session_id.is_none()) {
        if let Some(s) = live.iter().find(|s| Some(&s.id) == r.run_id.as_ref()) {
            r.session_id = Some(s.session_id.clone());
            changed = true;
        }
    }
    changed
}

/// ¿Hay algo que requiera consultar `claude agents`? (cola, o sessionId pendiente reciente).
pub fn needs_tick(runs: &[IssueRun], now: i64) -> bool {
    runs.iter().any(|r| {
        r.status == IssueRunStatus::Queued
            || (r.status == IssueRunStatus::Launched
                && r.session_id.is_none()
                && r.launched_at.is_some_and(|t| now - t < SESSION_LOOKUP_MS))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(issue: &str, status: IssueRunStatus, queued_at: i64) -> IssueRun {
        IssueRun {
            issue_id: issue.into(),
            identifier: format!("ID-{issue}"),
            workflow: "linear-issue".into(),
            run_id: None,
            session_id: None,
            cwd: "/repo".into(),
            queued_at,
            launched_at: None,
            status,
            error: None,
            options: LaunchOptions::default(),
        }
    }

    fn launched(issue: &str, run_id: &str, launched_at: i64) -> IssueRun {
        IssueRun {
            run_id: Some(run_id.into()),
            launched_at: Some(launched_at),
            ..run(issue, IssueRunStatus::Launched, launched_at - 1)
        }
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
            waiting_for: None,
        }
    }

    const NOW: i64 = 10_000_000;

    #[test]
    fn queue_is_fifo_and_respects_free_slots() {
        let runs = vec![
            run("c", IssueRunStatus::Queued, 30),
            run("a", IssueRunStatus::Queued, 10),
            run("b", IssueRunStatus::Queued, 20),
            run("x", IssueRunStatus::Failed, 1),
        ];
        let lv = vec![live("aa11", "working"), live("bb22", "done"), live("cc33", "stopped")];
        // 3 slots, 1 working ajeno → 2 libres: los dos más viejos.
        assert_eq!(next_to_launch(&runs, &lv, 3, NOW), vec![1, 2]);
        assert_eq!(next_to_launch(&runs, &lv, 1, NOW), Vec::<usize>::new());
        assert_eq!(next_to_launch(&runs, &[], 10, NOW), vec![1, 2, 0]);
    }

    #[test]
    fn launching_and_fresh_launches_occupy_slots() {
        let runs = vec![
            run("q", IssueRunStatus::Queued, 5),
            run("l", IssueRunStatus::Launching, 1),
            // Recién lanzado, todavía no figura en claude agents: ocupa.
            launched("f", "ffff0001", NOW - 1_000),
            // Lanzado hace mucho y no figura: no ocupa.
            launched("o", "ffff0002", NOW - LAUNCH_GRACE_MS - 1),
            // Figura y terminó: no ocupa (ni doble cuenta).
            launched("d", "dddd0001", NOW - 1_000),
        ];
        let lv = vec![live("dddd0001", "done")];
        assert_eq!(occupied_slots(&runs, &lv, NOW), 2);
        assert_eq!(next_to_launch(&runs, &lv, 2, NOW), Vec::<usize>::new());
        assert_eq!(next_to_launch(&runs, &lv, 3, NOW), vec![0]);
        // Si figura y está working, cuenta una sola vez (por claude agents).
        let lv = vec![live("ffff0001", "working"), live("dddd0001", "done")];
        assert_eq!(occupied_slots(&runs, &lv, NOW), 2);
    }

    #[test]
    fn session_ids_are_filled_from_live_runs() {
        let mut runs = vec![launched("a", "aaaa0001", NOW), run("q", IssueRunStatus::Queued, 1)];
        assert!(needs_tick(&runs, NOW));
        assert!(!fill_session_ids(&mut runs, &[live("zzzz", "working")]));
        assert!(fill_session_ids(&mut runs, &[live("aaaa0001", "working")]));
        assert_eq!(runs[0].session_id.as_deref(), Some("sess-aaaa0001"));
        runs.remove(1);
        assert!(!needs_tick(&runs, NOW));
        // Uno viejo sin sessionId ya no dispara consultas.
        let old = vec![launched("b", "bbbb", NOW - SESSION_LOOKUP_MS - 1)];
        assert!(!needs_tick(&old, NOW));
    }

    #[test]
    fn active_detection() {
        let lv = vec![live("aaaa", "working"), live("bbbb", "done")];
        assert!(is_active(&run("q", IssueRunStatus::Queued, 1), None, NOW));
        assert!(!is_active(&run("f", IssueRunStatus::Failed, 1), Some(&lv), NOW));
        assert!(is_active(&launched("a", "aaaa", 1), Some(&lv), NOW));
        assert!(!is_active(&launched("b", "bbbb", NOW), Some(&lv), NOW));
        assert!(is_active(&launched("c", "cccc", NOW - 10), Some(&lv), NOW));
        assert!(!is_active(&launched("c", "cccc", NOW - LAUNCH_GRACE_MS), Some(&lv), NOW));
    }

    #[test]
    fn blocked_sessions_stay_active_and_hold_a_slot() {
        // `state: "blocked"` = esperando un permiso o input: sigue viva.
        let lv = vec![live("aaaa", "blocked"), live("bbbb", "failed")];
        assert!(is_active(&launched("a", "aaaa", 1), Some(&lv), NOW));
        assert!(!is_active(&launched("b", "bbbb", 1), Some(&lv), NOW));
        assert_eq!(occupied_slots(&[], &lv, NOW), 1);
    }

    fn temp_file(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("agent-desk-issue-runs-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d.join("nested").join(FILE_NAME)
    }

    #[test]
    fn persistence_roundtrip_and_recovery() {
        let path = temp_file("roundtrip");
        assert_eq!(load_from(&path).unwrap(), StoreData::default());

        let mut l = launched("a", "aaaa0001", 123);
        l.session_id = Some("sess".into());
        let data = StoreData {
            runs: vec![run("q", IssueRunStatus::Queued, 5), l, run("x", IssueRunStatus::Launching, 7)],
        };
        save_to(&path, &data).unwrap();
        let back = load_from(&path).unwrap();
        // La cola sobrevive; el "launching" interrumpido se marca como fallido.
        assert_eq!(back.runs[0], data.runs[0]);
        assert_eq!(back.runs[1], data.runs[1]);
        assert_eq!(back.runs[2].status, IssueRunStatus::Failed);
        assert!(back.runs[2].error.as_deref().unwrap().contains("closed while"));

        // camelCase en disco, como el resto de la app.
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"issueId\"") && raw.contains("\"queued\""), "{raw}");

        std::fs::write(&path, "{nope").unwrap();
        assert!(load_from(&path).unwrap_err().contains("corrupt"));
        std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap()).unwrap();
    }

    #[test]
    fn prune_keeps_queued() {
        let mut data = StoreData::default();
        data.runs.push(run("q", IssueRunStatus::Queued, 0));
        for i in 1..=(MAX_HISTORY as i64 + 20) {
            data.runs.push(run(&i.to_string(), IssueRunStatus::Failed, i));
        }
        prune(&mut data);
        assert!(data.runs.len() <= MAX_HISTORY + 1);
        assert!(data.runs.iter().any(|r| r.status == IssueRunStatus::Queued));
        assert_eq!(data.runs[0].queued_at, MAX_HISTORY as i64 + 20);
    }

    #[test]
    fn prompt_format() {
        assert_eq!(run("a", IssueRunStatus::Queued, 0).prompt(), "/linear-issue ID-a");
    }
}
