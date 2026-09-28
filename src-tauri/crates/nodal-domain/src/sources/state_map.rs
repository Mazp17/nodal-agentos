//! State mapping between the provider and Nodal. All pure.
//!
//! - Pull (external → Nodal): triage/backlog → Backlog, unstarted → Todo, started → In Review
//!   or Blocked if the name says "review"/"block" (otherwise In Progress), completed → Done,
//!   canceled → Canceled. `unknown` ones (providers without types) go by name.
//! - Push (Nodal → external): only Todo, In Progress, In Review and Blocked. They go to the
//!   state with the same name or, if there is none, to the closest one of the same type
//!   (`started`; `unstarted` for Todo, which the queue pushes when a queued run is canceled)
//!   whose name justifies it; with no equivalent they stay on "Don't sync" (`None`: comment
//!   only). A confirmed mapping with no row for a status (e.g. Todo in one saved earlier)
//!   pushes nothing.
//! - Each report row marks its origin: suggested, confirmed or unmapped.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::model::{ExtKind, ExternalState, StateMap, TaskStatus};

/// Nodal statuses that are pushed to the provider.
pub const PUSHED: [TaskStatus; 4] = [
    TaskStatus::Todo,
    TaskStatus::InProgress,
    TaskStatus::InReview,
    TaskStatus::Blocked,
];

/// Lowercase, without accents or punctuation: "In-Review " → "inreview".
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

/// Pull proposal for an external state.
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
            if n.contains("done")
                || n.contains("complete")
                || n.contains("closed")
                || n.contains("hecho")
            {
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

/// Push proposal for a Nodal status. `None` = "Don't sync".
pub fn propose_push(status: TaskStatus, states: &[ExternalState]) -> Option<String> {
    let want = norm(status_name(status));
    // 1. Same name (ignoring case, spaces and the multi-team "ENG · " prefix).
    if let Some(s) = states
        .iter()
        .find(|s| norm(s.name.rsplit('·').next().unwrap_or(&s.name)) == want)
    {
        return Some(s.id.clone());
    }
    // 2. The closest one of its type: `started` for In Progress, In Review and Blocked (or
    //    `unknown` in providers without types); for Todo, the first one pull maps to Todo
    //    (`unstarted`).
    let candidates = states
        .iter()
        .filter(|s| matches!(s.kind, ExtKind::Started | ExtKind::Unknown));
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

/// Mapping proposed from scratch (pending confirmation).
pub fn propose(states: &[ExternalState]) -> StateMap {
    StateMap {
        pull: states
            .iter()
            .map(|s| (s.id.clone(), propose_pull(s)))
            .collect(),
        push: PUSHED
            .iter()
            .map(|st| (*st, propose_push(*st, states)))
            .collect(),
        confirmed_at: None,
        known_states: states.to_vec(),
    }
}

/// States added and removed relative to `known`, by id.
pub fn diff_known(
    known: &[ExternalState],
    current: &[ExternalState],
) -> (Vec<ExternalState>, Vec<ExternalState>) {
    let added = current
        .iter()
        .filter(|c| !known.iter().any(|k| k.id == c.id))
        .cloned()
        .collect();
    let removed = known
        .iter()
        .filter(|k| !current.iter().any(|c| c.id == k.id))
        .cloned()
        .collect();
    (added, removed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MapOrigin {
    Suggested,
    Confirmed,
    Unmapped,
}

/// Response of `source_states` (mirror of `SourceStatesReport` in `api.ts`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceStatesReport {
    pub states: Vec<ExternalState>,
    /// Saved mapping, filled in with the proposal for whatever is missing.
    pub proposal: StateMap,
    pub pull_origin: BTreeMap<String, MapOrigin>,
    pub push_origin: BTreeMap<TaskStatus, MapOrigin>,
    pub added: Vec<ExternalState>,
    pub removed: Vec<ExternalState>,
}

/// Fills in the saved mapping with the proposal and marks the origin of each row:
/// - unconfirmed map: everything is "suggested";
/// - confirmed: what was saved is "confirmed"; what is missing (a new state, or a push to a
///   state that disappeared) is "unmapped", with the proposal as its value.
pub fn report(saved: &StateMap, current: Vec<ExternalState>) -> SourceStatesReport {
    let confirmed = saved.confirmed_at.is_some();
    let kept = if confirmed {
        MapOrigin::Confirmed
    } else {
        MapOrigin::Suggested
    };
    let missing = if confirmed {
        MapOrigin::Unmapped
    } else {
        MapOrigin::Suggested
    };
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
    let statuses: Vec<TaskStatus> = PUSHED
        .iter()
        .copied()
        .chain(saved.push.keys().copied())
        .collect();
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
            known_states: if confirmed {
                saved.known_states.clone()
            } else {
                current.clone()
            },
        },
        states: current,
        pull_origin,
        push_origin,
        added,
        removed,
    }
}

/// Nodal status for an external state per the saved mapping. `None` = unmapped.
pub fn pull_status(map: &StateMap, state: &ExternalState) -> Option<TaskStatus> {
    map.pull.get(&state.id).copied()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// Unconfirmed mapping.
    Pending,
    /// "Don't sync".
    NoSync,
    /// The Nodal status is not in the push mapping.
    NotMapped,
    /// The target external state no longer exists in the provider.
    TargetGone(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushTarget {
    Push(String),
    Skip(SkipReason),
}

/// Push target for `status`. `current`: the provider's current states, if known (so as not
/// to push to one that disappeared).
pub fn push_target(
    map: &StateMap,
    status: TaskStatus,
    current: Option<&[ExternalState]>,
) -> PushTarget {
    if map.confirmed_at.is_none() {
        return PushTarget::Skip(SkipReason::Pending);
    }
    match map.push.get(&status) {
        None => PushTarget::Skip(SkipReason::NotMapped),
        Some(None) => PushTarget::Skip(SkipReason::NoSync),
        Some(Some(id)) => literal_target(map, id, current),
    }
}

/// Push to an explicit external id (outbox with an id that is not a Nodal status).
pub fn literal_target(map: &StateMap, id: &str, current: Option<&[ExternalState]>) -> PushTarget {
    if map.confirmed_at.is_none() {
        return PushTarget::Skip(SkipReason::Pending);
    }
    match current {
        Some(states) if !states.iter().any(|s| s.id == id) => {
            PushTarget::Skip(SkipReason::TargetGone(id.to_string()))
        }
        _ => PushTarget::Push(id.to_string()),
    }
}

/// Validates a mapping before saving it: push targets must exist.
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

#[cfg(any(test, feature = "test-support"))]
pub mod tests;
