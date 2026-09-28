//! Pure sync decisions: what to do with a pulled item, a project change or an outbox row.
//! The worker pass (network, database) lives in nodal-app.

use serde::Serialize;

use crate::model::providers::{ErrorKind, ExternalItem, ProviderError};
use crate::model::{
    ExtProject, ExternalState, MovedInfo, OutboxPayload, SourceLink, StateMap, Task, TaskStatus,
};

use super::routing::{project_rule, suggest_repo};
use super::state_map::{literal_target, pull_status, push_target, PushTarget, SkipReason};

pub const BACKOFF_BASE_MS: i64 = 30_000;
pub const BACKOFF_MAX_MS: i64 = 60 * 60_000;
/// Attempts for an outbox row with transient errors before dropping it.
pub const MAX_ATTEMPTS: i64 = 10;
/// Each scope's states are re-read every so often (or on a manual sync), not on every pass.
pub const STATES_TTL_MS: i64 = 10 * 60_000;
/// Pause of the whole provider after a rate limit or a rejected key (the worker leaves it
/// alone until then; a manual sync or a new key lifts it).
pub const RATE_LIMIT_PAUSE_MS: i64 = 5 * 60_000;
pub const AUTH_PAUSE_MS: i64 = 30 * 60_000;

/// Wait before attempt `attempts + 1` (30 s, 1 min, 2 min… up to 1 h).
pub fn backoff_ms(attempts: i64) -> i64 {
    let exp = (attempts - 1).clamp(0, 20) as u32;
    BACKOFF_BASE_MS.saturating_mul(1 << exp).min(BACKOFF_MAX_MS)
}

/// Paused provider (see `RATE_LIMIT_PAUSE_MS`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pause {
    pub until: i64,
    pub reason: String,
}

/// Errors that belong to the whole provider rather than one item: they cut the pass short and
/// pause it.
pub fn halts(e: &ProviderError) -> bool {
    matches!(e.kind, ErrorKind::RateLimited | ErrorKind::Auth)
}

/// Mirror of `SyncReport` in `api.ts`.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    /// Tasks refreshed from the provider.
    pub pulled: usize,
    /// Writes made to the provider (states and comments).
    pub pushed: usize,
    /// New tasks from auto-import.
    pub imported: usize,
    pub errors: Vec<String>,
    /// Notices that are not errors: new or vanished states, items with no repo for
    /// auto-import, pushes dropped because the target no longer exists.
    pub notices: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PullDecision {
    pub title: Option<String>,
    pub status: Option<TaskStatus>,
    /// Record the external state as seen. An unmapped one is not recorded: once the user
    /// maps it, the next pull will see it as a change and apply it. Nor while there is a
    /// pending status push: if that push is dropped, the next pull applies the external one.
    pub record_state: bool,
    /// The external state is not in the pull mapping.
    pub unmapped: bool,
    pub sync_error: Option<String>,
}

/// `pending_push`: the task has a `set_state` in the outbox (a local change not yet pushed),
/// which wins over the external state just like an active run.
pub fn decide_pull(
    task: &Task,
    item: &ExternalItem,
    map: &StateMap,
    active_run: bool,
    pending_push: bool,
) -> PullDecision {
    let prev = task
        .source
        .as_ref()
        .and_then(|s| s.external_state.as_ref())
        .map(|s| s.id.as_str());
    let changed = prev != Some(item.state.id.as_str());
    let mapped = pull_status(map, &item.state);
    let apply = changed && !active_run && !pending_push;
    PullDecision {
        title: (item.title != task.title).then(|| item.title.clone()),
        status: if apply {
            mapped.filter(|s| *s != task.status)
        } else {
            None
        },
        record_state: mapped.is_some() && !pending_push,
        unmapped: mapped.is_none(),
        sync_error: mapped.is_none().then(|| {
            format!(
                "External state \"{}\" is not mapped to a Nodal status. Review the mapping.",
                item.state.name
            )
        }),
    }
}

/// A task's project, origin rule and project-change notice after the pull.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectUpdate {
    pub project: Option<ExtProject>,
    pub rule_id: Option<String>,
    pub moved: Option<MovedInfo>,
}

/// A task that came in through a project rule and whose issue is now in another project does
/// not change repo: it stays `moved` until the user decides. If the repo it would get is the
/// same, there is nothing to decide (it switches to the new project's rule, if any); if it
/// returns to the original project, the notice is cleared. Other tasks only record the project.
pub fn decide_project(task: &Task, item: &ExternalItem, link: &SourceLink) -> ProjectUpdate {
    let src = task.source.as_ref();
    let current = item.project();
    let current_id = current.as_ref().map(|p| p.id.as_str());
    let rule_id = src.and_then(|s| s.rule_id.clone());
    let anchor = match src.and_then(|s| s.moved.as_ref()) {
        Some(m) => Some(m.from_project.clone()),
        None => rule_id
            .as_deref()
            .and_then(|id| {
                link.repo_rules
                    .iter()
                    .find(|r| r.id == id && r.is_project())
            })
            .map(|r| {
                let stored = src
                    .and_then(|s| s.project.as_ref())
                    .filter(|p| p.id == r.value);
                ExtProject {
                    id: r.value.clone(),
                    name: stored.map_or_else(|| r.name.clone(), |p| p.name.clone()),
                }
            }),
    };
    let Some(anchor) = anchor.filter(|a| current_id != Some(a.id.as_str())) else {
        return ProjectUpdate {
            project: current,
            rule_id,
            moved: None,
        };
    };
    let suggested = suggest_repo(link, current_id, &item.labels);
    if suggested.as_deref() == Some(task.repo_id.as_str()) {
        let rule_id = project_rule(link, current_id)
            .filter(|r| r.repo_id == task.repo_id)
            .map(|r| r.id.clone());
        return ProjectUpdate {
            project: current,
            rule_id,
            moved: None,
        };
    }
    ProjectUpdate {
        project: current.clone(),
        rule_id,
        moved: Some(MovedInfo {
            from_project: anchor,
            to_project: current,
            suggested_repo_id: suggested,
        }),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PushAction {
    Comment(String),
    SetState(String),
    /// The row is dropped; with a message if the user must be notified.
    Drop(Option<String>),
}

/// What to do with an outbox row. `map`: the one of the task's link (`None` = task without a
/// link, same as a pending mapping). `current`: the scope's current states, if known.
pub fn decide_push(
    payload: &OutboxPayload,
    map: Option<&StateMap>,
    current: Option<&[ExternalState]>,
    external_state: Option<&ExternalState>,
) -> PushAction {
    let state_id = match payload {
        OutboxPayload::Comment { body } => return PushAction::Comment(body.clone()),
        OutboxPayload::SetState { state_id } => state_id,
    };
    let Some(map) = map else {
        return PushAction::Drop(None);
    };
    let target = match TaskStatus::parse(state_id) {
        Some(status) => push_target(map, status, current),
        None => literal_target(map, state_id, current),
    };
    match target {
        PushTarget::Push(id) if external_state.is_some_and(|s| s.id == id) => {
            PushAction::Drop(None)
        }
        PushTarget::Push(id) => PushAction::SetState(id),
        PushTarget::Skip(SkipReason::TargetGone(id)) => {
            let name = map
                .known_states
                .iter()
                .find(|s| s.id == id)
                .map_or(id.as_str(), |s| s.name.as_str());
            PushAction::Drop(Some(format!(
                "The mapped state \"{name}\" no longer exists in the provider; the status was not pushed. Review the mapping."
            )))
        }
        PushTarget::Skip(_) => PushAction::Drop(None),
    }
}

/// Invisible marker (an HTML comment) that identifies the outbox row in the comment body.
/// Outbox ids are `AUTOINCREMENT`: they are never reused.
pub fn outbox_marker(id: i64) -> String {
    format!("<!-- nodal:outbox:{id} -->")
}

/// Body sent: the text plus the marker.
pub fn marked_body(body: &str, id: i64) -> String {
    format!("{}\n\n{}", body.trim_end(), outbox_marker(id))
}

#[cfg(test)]
mod tests;
