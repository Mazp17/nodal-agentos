//! Mapeo de estados entre el proveedor y Nodal. Todo puro.
//!
//! - Pull (externo → Nodal): triage/backlog → Backlog, unstarted → Todo, started → In Review
//!   o Blocked si el nombre dice "review"/"block" (si no, In Progress), completed → Done,
//!   canceled → Canceled. Los `unknown` (proveedores sin tipos) van por nombre.
//! - Push (Nodal → externo): solo Todo, In Progress, In Review y Blocked. Van al estado del
//!   mismo nombre o, si no hay, al más parecido del mismo tipo (`started`; `unstarted` para
//!   Todo, que la cola empuja al cancelar un run en cola) cuyo nombre lo justifique; sin
//!   equivalente queda en "No sincronizar" (`None`: solo comentario). Un mapeo confirmado
//!   sin fila para un estado (p. ej. Todo en uno guardado antes) no empuja nada.
//! - Cada fila del reporte marca su origen: sugerido, confirmado o sin mapear.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::domain::{ExtKind, ExternalState, StateMap, TaskStatus};

/// Estados Nodal que se empujan al proveedor.
pub const PUSHED: [TaskStatus; 4] =
    [TaskStatus::Todo, TaskStatus::InProgress, TaskStatus::InReview, TaskStatus::Blocked];

/// Minúsculas, sin acentos ni signos: "In-Review " → "inreview".
fn norm(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'ä' => 'a',
            'é' | 'è' | 'ë' => 'e',
            'í' | 'ì' | 'ï' => 'i',
            'ó' | 'ò' | 'ö' => 'o',
            'ú' | 'ù' | 'ü' => 'u',
            other => other,
        })
        .filter(|c| c.is_alphanumeric())
        .collect()
}

fn says_review(n: &str) -> bool {
    n.contains("review") || n.contains("revision") || n.contains("qa")
}

fn says_block(n: &str) -> bool {
    n.contains("block") || n.contains("bloque")
}

/// Propuesta pull para un estado externo.
pub fn propose_pull(state: &ExternalState) -> TaskStatus {
    let n = norm(&state.name);
    match state.kind {
        ExtKind::Triage | ExtKind::Backlog => TaskStatus::Backlog,
        ExtKind::Unstarted => TaskStatus::Todo,
        ExtKind::Started if says_review(&n) => TaskStatus::InReview,
        ExtKind::Started if says_block(&n) => TaskStatus::Blocked,
        ExtKind::Started => TaskStatus::InProgress,
        ExtKind::Completed => TaskStatus::Done,
        ExtKind::Canceled => TaskStatus::Canceled,
        ExtKind::Unknown => {
            if n.contains("done") || n.contains("complete") || n.contains("closed") || n.contains("hecho") {
                TaskStatus::Done
            } else if n.contains("cancel") || n.contains("duplicate") {
                TaskStatus::Canceled
            } else if says_review(&n) {
                TaskStatus::InReview
            } else if says_block(&n) {
                TaskStatus::Blocked
            } else if n.contains("progress") || n.contains("doing") || n.contains("curso") {
                TaskStatus::InProgress
            } else if n.contains("todo") || n.contains("next") || n.contains("ready") {
                TaskStatus::Todo
            } else {
                TaskStatus::Backlog
            }
        }
    }
}

fn status_name(s: TaskStatus) -> &'static str {
    match s {
        TaskStatus::Backlog => "Backlog",
        TaskStatus::Todo => "Todo",
        TaskStatus::InProgress => "In Progress",
        TaskStatus::InReview => "In Review",
        TaskStatus::Blocked => "Blocked",
        TaskStatus::Done => "Done",
        TaskStatus::Canceled => "Canceled",
    }
}

/// Propuesta push para un estado Nodal. `None` = "No sincronizar".
pub fn propose_push(status: TaskStatus, states: &[ExternalState]) -> Option<String> {
    let want = norm(status_name(status));
    // 1. Mismo nombre (ignorando mayúsculas, espacios y el prefijo "ENG · " de multi-team).
    if let Some(s) = states.iter().find(|s| norm(s.name.rsplit('·').next().unwrap_or(&s.name)) == want) {
        return Some(s.id.clone());
    }
    // 2. El más parecido de su tipo: `started` para In Progress, In Review y Blocked (o
    //    `unknown` en proveedores sin tipos); Todo, el primero que el pull mapea a Todo
    //    (`unstarted`).
    let candidates = states.iter().filter(|s| matches!(s.kind, ExtKind::Started | ExtKind::Unknown));
    let found = match status {
        TaskStatus::InReview => candidates.clone().find(|s| says_review(&norm(&s.name))),
        TaskStatus::Blocked => candidates.clone().find(|s| says_block(&norm(&s.name))),
        TaskStatus::InProgress => candidates
            .clone()
            .find(|s| norm(&s.name).contains("progress"))
            .or_else(|| {
                candidates.clone().find(|s| {
                    let n = norm(&s.name);
                    s.kind == ExtKind::Started && !says_review(&n) && !says_block(&n)
                })
            }),
        _ => states.iter().find(|s| propose_pull(s) == status),
    };
    found.map(|s| s.id.clone())
}

/// Mapeo propuesto desde cero (pendiente de confirmar).
pub fn propose(states: &[ExternalState]) -> StateMap {
    StateMap {
        pull: states.iter().map(|s| (s.id.clone(), propose_pull(s))).collect(),
        push: PUSHED.iter().map(|st| (*st, propose_push(*st, states))).collect(),
        confirmed_at: None,
        known_states: states.to_vec(),
    }
}

/// Estados nuevos y desaparecidos respecto de `known`, por id.
pub fn diff_known(known: &[ExternalState], current: &[ExternalState]) -> (Vec<ExternalState>, Vec<ExternalState>) {
    let added = current.iter().filter(|c| !known.iter().any(|k| k.id == c.id)).cloned().collect();
    let removed = known.iter().filter(|k| !current.iter().any(|c| c.id == k.id)).cloned().collect();
    (added, removed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MapOrigin {
    Suggested,
    Confirmed,
    Unmapped,
}

/// Respuesta de `source_states` (espejo de `SourceStatesReport` en `api.ts`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceStatesReport {
    pub states: Vec<ExternalState>,
    /// Mapeo guardado completado con la propuesta para lo que falte.
    pub proposal: StateMap,
    pub pull_origin: BTreeMap<String, MapOrigin>,
    pub push_origin: BTreeMap<TaskStatus, MapOrigin>,
    pub added: Vec<ExternalState>,
    pub removed: Vec<ExternalState>,
}

/// Completa el mapeo guardado con la propuesta y marca el origen de cada fila:
/// - mapa sin confirmar: todo es "sugerido";
/// - confirmado: lo guardado es "confirmado"; lo que falta (estado nuevo, o push hacia un
///   estado que desapareció) es "sin mapear", con la propuesta como valor.
pub fn report(saved: &StateMap, current: Vec<ExternalState>) -> SourceStatesReport {
    let confirmed = saved.confirmed_at.is_some();
    let kept = if confirmed { MapOrigin::Confirmed } else { MapOrigin::Suggested };
    let missing = if confirmed { MapOrigin::Unmapped } else { MapOrigin::Suggested };
    let (added, removed) = diff_known(&saved.known_states, &current);

    let mut pull = BTreeMap::new();
    let mut pull_origin = BTreeMap::new();
    for s in &current {
        let (value, origin) = match saved.pull.get(&s.id) {
            Some(v) => (*v, kept),
            None => (propose_pull(s), missing),
        };
        pull.insert(s.id.clone(), value);
        pull_origin.insert(s.id.clone(), origin);
    }

    let mut push = BTreeMap::new();
    let mut push_origin = BTreeMap::new();
    let statuses: Vec<TaskStatus> = PUSHED.iter().copied().chain(saved.push.keys().copied()).collect();
    for st in statuses {
        if push.contains_key(&st) {
            continue;
        }
        let (value, origin) = match saved.push.get(&st) {
            Some(None) => (None, kept),
            Some(Some(id)) if current.iter().any(|s| &s.id == id) => (Some(id.clone()), kept),
            _ => (propose_push(st, &current), missing),
        };
        push.insert(st, value);
        push_origin.insert(st, origin);
    }

    SourceStatesReport {
        proposal: StateMap {
            pull,
            push,
            confirmed_at: saved.confirmed_at,
            known_states: if confirmed { saved.known_states.clone() } else { current.clone() },
        },
        states: current,
        pull_origin,
        push_origin,
        added,
        removed,
    }
}

/// Estado Nodal para un estado externo según el mapeo guardado. `None` = sin mapear.
pub fn pull_status(map: &StateMap, state: &ExternalState) -> Option<TaskStatus> {
    map.pull.get(&state.id).copied()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// Mapeo sin confirmar.
    Pending,
    /// "No sincronizar".
    NoSync,
    /// El estado Nodal no está en el mapeo push.
    NotMapped,
    /// El estado externo destino ya no existe en el proveedor.
    TargetGone(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushTarget {
    Push(String),
    Skip(SkipReason),
}

/// Destino del push de `status`. `current`: estados actuales del proveedor si se conocen
/// (para no empujar a uno que desapareció).
pub fn push_target(map: &StateMap, status: TaskStatus, current: Option<&[ExternalState]>) -> PushTarget {
    if map.confirmed_at.is_none() {
        return PushTarget::Skip(SkipReason::Pending);
    }
    match map.push.get(&status) {
        None => PushTarget::Skip(SkipReason::NotMapped),
        Some(None) => PushTarget::Skip(SkipReason::NoSync),
        Some(Some(id)) => literal_target(map, id, current),
    }
}

/// Push a un id externo explícito (outbox con un id que no es un estado Nodal).
pub fn literal_target(map: &StateMap, id: &str, current: Option<&[ExternalState]>) -> PushTarget {
    if map.confirmed_at.is_none() {
        return PushTarget::Skip(SkipReason::Pending);
    }
    match current {
        Some(states) if !states.iter().any(|s| s.id == id) => PushTarget::Skip(SkipReason::TargetGone(id.to_string())),
        _ => PushTarget::Push(id.to_string()),
    }
}

/// Valida un mapeo antes de guardarlo: los destinos push tienen que existir.
pub fn validate(map: &StateMap, current: &[ExternalState]) -> Result<(), String> {
    for (st, target) in &map.push {
        if let Some(id) = target {
            if !current.iter().any(|s| &s.id == id) {
                return Err(format!(
                    "The state mapped for {} no longer exists in the provider. Pick another one.",
                    status_name(*st)
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
pub mod tests {
    use super::*;

    fn st(id: &str, name: &str, kind: ExtKind) -> ExternalState {
        ExternalState { id: id.into(), name: name.into(), kind, color: Some("#999999".into()) }
    }

    /// Team ficticio con los estados típicos de un workspace de Linear.
    pub fn team_states() -> Vec<ExternalState> {
        vec![
            st("s-triage", "Triage", ExtKind::Triage),
            st("s-backlog", "Backlog", ExtKind::Backlog),
            st("s-todo", "Todo", ExtKind::Unstarted),
            st("s-progress", "In Progress", ExtKind::Started),
            st("s-review", "In Review", ExtKind::Started),
            st("s-blocked", "Blocked", ExtKind::Started),
            st("s-done", "Done", ExtKind::Completed),
            st("s-canceled", "Canceled", ExtKind::Canceled),
            st("s-dup", "Duplicate", ExtKind::Canceled),
        ]
    }

    #[test]
    fn proposal_for_typical_team() {
        let m = propose(&team_states());
        let pull = |id: &str| m.pull[id];
        assert_eq!(pull("s-triage"), TaskStatus::Backlog);
        assert_eq!(pull("s-backlog"), TaskStatus::Backlog);
        assert_eq!(pull("s-todo"), TaskStatus::Todo);
        assert_eq!(pull("s-progress"), TaskStatus::InProgress);
        assert_eq!(pull("s-review"), TaskStatus::InReview);
        assert_eq!(pull("s-blocked"), TaskStatus::Blocked);
        assert_eq!(pull("s-done"), TaskStatus::Done);
        assert_eq!(pull("s-canceled"), TaskStatus::Canceled);
        assert_eq!(pull("s-dup"), TaskStatus::Canceled);

        assert_eq!(m.push.len(), 4);
        assert_eq!(m.push[&TaskStatus::Todo].as_deref(), Some("s-todo"));
        assert_eq!(m.push[&TaskStatus::InProgress].as_deref(), Some("s-progress"));
        assert_eq!(m.push[&TaskStatus::InReview].as_deref(), Some("s-review"));
        assert_eq!(m.push[&TaskStatus::Blocked].as_deref(), Some("s-blocked"));
        assert!(m.confirmed_at.is_none());
        assert_eq!(m.known_states.len(), 9);
    }

    #[test]
    fn push_falls_back_to_similar_or_no_sync() {
        // Sin "Blocked" ni "In Review": In Progress va al started más parecido, Blocked y
        // In Review quedan en "No sincronizar".
        let states = vec![
            st("a", "Todo", ExtKind::Unstarted),
            st("b", "Doing", ExtKind::Started),
            st("c", "Code review", ExtKind::Started),
            st("d", "Done", ExtKind::Completed),
        ];
        assert_eq!(propose_push(TaskStatus::InProgress, &states).as_deref(), Some("b"));
        assert_eq!(propose_push(TaskStatus::InReview, &states).as_deref(), Some("c"));
        assert_eq!(propose_push(TaskStatus::Blocked, &states), None);

        let unstarted = vec![st("u", "Ready", ExtKind::Unstarted), st("b", "Doing", ExtKind::Started)];
        assert_eq!(propose_push(TaskStatus::Todo, &unstarted).as_deref(), Some("u"));
        assert_eq!(propose_push(TaskStatus::Todo, &[st("b", "Doing", ExtKind::Started)]), None);

        let only_review = vec![st("r", "Peer Review", ExtKind::Started)];
        assert_eq!(propose_push(TaskStatus::InProgress, &only_review), None);
    }

    #[test]
    fn push_matches_names_with_team_prefix() {
        let states = vec![st("x", "ENG · Blocked", ExtKind::Started), st("y", "ENG · In Progress", ExtKind::Started)];
        assert_eq!(propose_push(TaskStatus::Blocked, &states).as_deref(), Some("x"));
        assert_eq!(propose_push(TaskStatus::InProgress, &states).as_deref(), Some("y"));
    }

    #[test]
    fn unknown_kinds_go_by_name() {
        let pull = |n: &str| propose_pull(&st("x", n, ExtKind::Unknown));
        assert_eq!(pull("Done"), TaskStatus::Done);
        assert_eq!(pull("Waiting for review"), TaskStatus::InReview);
        assert_eq!(pull("Blocked"), TaskStatus::Blocked);
        assert_eq!(pull("Doing"), TaskStatus::InProgress);
        assert_eq!(pull("To do"), TaskStatus::Todo);
        assert_eq!(pull("Ideas"), TaskStatus::Backlog);
    }

    #[test]
    fn report_pending_map_is_all_suggested() {
        let states = team_states();
        let saved = propose(&states);
        let r = report(&saved, states.clone());
        assert!(r.pull_origin.values().all(|o| *o == MapOrigin::Suggested));
        assert!(r.push_origin.values().all(|o| *o == MapOrigin::Suggested));
        assert!(r.added.is_empty() && r.removed.is_empty());
        assert_eq!(r.proposal.pull, saved.pull);
    }

    #[test]
    fn report_detects_new_and_removed_states() {
        let mut states = team_states();
        let mut saved = propose(&states);
        saved.confirmed_at = Some(1);
        // El usuario decidió no sincronizar Blocked.
        saved.push.insert(TaskStatus::Blocked, None);

        // Aparece "QA", desaparece "In Review".
        states.retain(|s| s.id != "s-review");
        states.push(st("s-qa", "QA", ExtKind::Started));
        let r = report(&saved, states);

        assert_eq!(r.added.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["s-qa"]);
        assert_eq!(r.removed.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["s-review"]);
        assert_eq!(r.pull_origin["s-qa"], MapOrigin::Unmapped);
        assert_eq!(r.proposal.pull["s-qa"], TaskStatus::InReview);
        assert_eq!(r.pull_origin["s-todo"], MapOrigin::Confirmed);
        assert!(!r.proposal.pull.contains_key("s-review"));
        // Push a In Review apuntaba a un estado que ya no está: sin mapear, con propuesta nueva.
        assert_eq!(r.push_origin[&TaskStatus::InReview], MapOrigin::Unmapped);
        assert_eq!(r.proposal.push[&TaskStatus::InReview].as_deref(), Some("s-qa"));
        assert_eq!(r.push_origin[&TaskStatus::Blocked], MapOrigin::Confirmed);
        assert_eq!(r.proposal.push[&TaskStatus::Blocked], None);
        assert_eq!(r.proposal.known_states, saved.known_states);
    }

    #[test]
    fn push_target_rules() {
        let states = team_states();
        let mut m = propose(&states);
        assert_eq!(push_target(&m, TaskStatus::InReview, None), PushTarget::Skip(SkipReason::Pending));
        m.confirmed_at = Some(1);
        assert_eq!(push_target(&m, TaskStatus::InReview, Some(&states)), PushTarget::Push("s-review".into()));
        assert_eq!(push_target(&m, TaskStatus::Done, Some(&states)), PushTarget::Skip(SkipReason::NotMapped));
        m.push.insert(TaskStatus::Blocked, None);
        assert_eq!(push_target(&m, TaskStatus::Blocked, None), PushTarget::Skip(SkipReason::NoSync));
        let gone: Vec<_> = states.iter().filter(|s| s.id != "s-review").cloned().collect();
        assert_eq!(
            push_target(&m, TaskStatus::InReview, Some(&gone)),
            PushTarget::Skip(SkipReason::TargetGone("s-review".into()))
        );
        // Sin estados actuales conocidos se empuja igual.
        assert_eq!(push_target(&m, TaskStatus::InReview, None), PushTarget::Push("s-review".into()));
    }

    #[test]
    fn validate_rejects_missing_targets() {
        let states = team_states();
        let mut m = propose(&states);
        assert!(validate(&m, &states).is_ok());
        m.push.insert(TaskStatus::InReview, Some("nope".into()));
        assert!(validate(&m, &states).unwrap_err().contains("In Review"));
    }
}
