//! Sync with the providers: a worker every 60 s and `sync_now`.
//!
//! Order per provider: current states of each link → push (drains the outbox) → pull →
//! auto-import. Push goes first so that pull already sees what Nodal pushed.
//!
//! - Pull: title and plan (unless overridden) always; the Nodal status only if the
//!   `external_state.id` changed and the task has no active run (Nodal wins). An unmapped
//!   state does not change the Nodal status and is left in `sync_error`.
//! - Push: exponential backoff per row; `set_state` rows with a pending mapping, "Don't
//!   sync", no mapping or targeting a vanished state are dropped (comments are sent
//!   anyway). Permanent errors or `MAX_ATTEMPTS` attempts drop the row; a rate limit or an
//!   invalid key spend no attempts and stop draining the provider for that pass.
//!   A failed row holds back the following rows of its task (order: status → comment).
//! - Pull only for open tasks or those closed less than 7 days ago; an unlinked item
//!   does not come back through auto-import.
//! - Auto-import: open items in the scope created after the link, with a repo from a rule or
//!   the default; without a resolvable repo they are not imported and it is reported. The full
//!   pull is only requested for new candidates with a repo. Each project rule adds (even if the
//!   link has no auto-import) the open items of its project created after the rule.
//! - Project: pull stores each task's provider project; one that came in through a project
//!   rule and changed project is marked `moved` (it does not move on its own).
//! - A rate limit or rejected key at any step cuts the provider's pass short and pauses it
//!   in memory (`SyncMemo`); each scope's states are re-read every `STATES_TTL_MS`. A
//!   manual sync (`force`) ignores the pause and re-reads the states.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::Connection;
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::db::{rows, with_db, Db, DbError};
use crate::util::now_ms;
use crate::domain::{
    ExtProject, ExternalState, MovedInfo, OutboxPayload, PlanRef, SourceLink, StateChanges, StateMap, Task, TaskStatus,
};

use super::import::{import_items, project_rule, rule_query, suggest_repo, suggest_repo_for};
use super::plan::{plan_path, render_plan, write_plan};
use super::state_map::{diff_known, literal_target, pull_status, push_target, PushTarget, SkipReason};
use super::{
    resolve, store, ErrorKind, ExternalItem, ImportQuery, PResult, Provider, ProviderError, ProviderResult,
    ProvidersState, TaskProvider,
};

pub const TICK: Duration = Duration::from_secs(60);
/// First pass a while after startup, so as not to compete with the rest of startup.
const FIRST_TICK: Duration = Duration::from_secs(15);
pub const BACKOFF_BASE_MS: i64 = 30_000;
pub const BACKOFF_MAX_MS: i64 = 60 * 60_000;
/// Attempts for an outbox row with transient errors before dropping it.
pub const MAX_ATTEMPTS: i64 = 10;
const AUTO_IMPORT_MAX_PAGES: usize = 4;
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

/// Sync memory between passes (in memory; lives under `ProvidersState::sync_lock`).
#[derive(Debug, Default)]
pub struct SyncMemo {
    /// Provider → pause.
    pub paused: HashMap<String, Pause>,
    /// Link → (read at, mapping they were read with, scope states). A different mapping
    /// (the user saved it) invalidates the entry: a freshly mapped target is not taken as
    /// vanished.
    states: HashMap<String, (i64, StateMap, Vec<ExternalState>)>,
}

/// Errors that belong to the whole provider rather than one item: they cut the pass short and
/// pause it.
fn halts(e: &ProviderError) -> bool {
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

// ---------- Pure decisions ----------

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
pub fn decide_pull(task: &Task, item: &ExternalItem, map: &StateMap, active_run: bool, pending_push: bool) -> PullDecision {
    let prev = task.source.as_ref().and_then(|s| s.external_state.as_ref()).map(|s| s.id.as_str());
    let changed = prev != Some(item.state.id.as_str());
    let mapped = pull_status(map, &item.state);
    let apply = changed && !active_run && !pending_push;
    PullDecision {
        title: (item.title != task.title).then(|| item.title.clone()),
        status: if apply { mapped.filter(|s| *s != task.status) } else { None },
        record_state: mapped.is_some() && !pending_push,
        unmapped: mapped.is_none(),
        sync_error: mapped.is_none().then(|| {
            format!("External state \"{}\" is not mapped to a Nodal status. Review the mapping.", item.state.name)
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
            .and_then(|id| link.repo_rules.iter().find(|r| r.id == id && r.is_project()))
            .map(|r| {
                let stored = src.and_then(|s| s.project.as_ref()).filter(|p| p.id == r.value);
                ExtProject { id: r.value.clone(), name: stored.map_or_else(|| r.name.clone(), |p| p.name.clone()) }
            }),
    };
    let Some(anchor) = anchor.filter(|a| current_id != Some(a.id.as_str())) else {
        return ProjectUpdate { project: current, rule_id, moved: None };
    };
    let suggested = suggest_repo(link, current_id, &item.labels);
    if suggested.as_deref() == Some(task.repo_id.as_str()) {
        let rule_id = project_rule(link, current_id).filter(|r| r.repo_id == task.repo_id).map(|r| r.id.clone());
        return ProjectUpdate { project: current, rule_id, moved: None };
    }
    ProjectUpdate {
        project: current.clone(),
        rule_id,
        moved: Some(MovedInfo { from_project: anchor, to_project: current, suggested_repo_id: suggested }),
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
    let Some(map) = map else { return PushAction::Drop(None) };
    let target = match TaskStatus::parse(state_id) {
        Some(status) => push_target(map, status, current),
        None => literal_target(map, state_id, current),
    };
    match target {
        PushTarget::Push(id) if external_state.is_some_and(|s| s.id == id) => PushAction::Drop(None),
        PushTarget::Push(id) => PushAction::SetState(id),
        PushTarget::Skip(SkipReason::TargetGone(id)) => {
            let name = map.known_states.iter().find(|s| s.id == id).map_or(id.as_str(), |s| s.name.as_str());
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

/// Sends the comment of row `id` unless it is already in the provider: if an earlier attempt
/// reached Linear but not `outbox_done` (timeout, app closed), it is not repeated.
async fn send_comment(p: &Provider, external_id: &str, body: &str, id: i64) -> ProviderResult<()> {
    if p.has_comment_with(external_id, &outbox_marker(id)).await? {
        return Ok(());
    }
    p.comment(external_id, &marked_body(body, id)).await
}

// ---------- Sync pass ----------

fn err(e: DbError) -> String {
    e.to_string()
}

/// A full pass (or only for `link_filter`). `providers`: those with a key.
/// `force` (manual sync): ignores the providers' pause and re-reads the states.
pub async fn sync_run(
    db: &Db,
    data_dir: &Path,
    providers: &[Provider],
    link_filter: Option<&str>,
    now: i64,
    memo: &mut SyncMemo,
    force: bool,
) -> SyncReport {
    let mut report = SyncReport::default();
    let links = match with_db(db, |c| store::list_links(c, None)).await {
        Ok(l) => l,
        Err(e) => {
            report.errors.push(err(e));
            return report;
        }
    };
    if let Some(id) = link_filter {
        match links.iter().find(|l| l.id == id) {
            None => {
                report.errors.push("Source not found.".into());
                return report;
            }
            Some(l) if !providers.iter().any(|p| p.name() == l.provider) => {
                report.errors.push(format!("The {} API key is missing. Add it in Settings → Providers.", l.provider));
                return report;
            }
            _ => {}
        }
    }

    for p in providers {
        let name = p.name();
        match memo.paused.get(name) {
            Some(pause) if !force && pause.until > now => {
                report.notices.push(format!(
                    "{name}: sync paused until {} ({})",
                    super::iso_from_ms(pause.until),
                    pause.reason
                ));
                continue;
            }
            Some(_) => {
                memo.paused.remove(name);
            }
            None => {}
        }
        let targets: Vec<&SourceLink> = links
            .iter()
            .filter(|l| l.provider == name && link_filter.is_none_or(|id| id == l.id))
            .collect();

        let mut current: HashMap<String, Vec<ExternalState>> = HashMap::new();
        // Errors attributable to each link in this pass (states, pull, auto-import).
        let mut link_errors: HashMap<String, Vec<String>> = HashMap::new();
        // Rate limit or rejected key: the rest of the pass would fail anyway.
        let mut halt: Option<ProviderError> = None;
        for link in &targets {
            if halt.is_some() {
                break;
            }
            let cached = memo
                .states
                .get(&link.id)
                .filter(|(at, map, _)| !force && now - at < STATES_TTL_MS && *map == link.state_map);
            if let Some((_, _, states)) = cached {
                current.insert(link.id.clone(), states.clone());
                continue;
            }
            match p.states(&link.scope).await {
                Ok(states) => {
                    memo.states.insert(link.id.clone(), (now, link.state_map.clone(), states.clone()));
                    if link.state_map.confirmed_at.is_some() {
                        let (added, removed) = diff_known(&link.state_map.known_states, &states);
                        let names = |v: &[ExternalState]| v.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", ");
                        if !added.is_empty() {
                            report.notices.push(format!("{}: new states to map: {}.", link.scope.name, names(&added)));
                        }
                        if !removed.is_empty() {
                            report.notices.push(format!("{}: states removed: {}.", link.scope.name, names(&removed)));
                        }
                        let (id, changes) = (link.id.clone(), StateChanges { added, removed });
                        if let Err(e) = with_db(db, move |c| store::set_pending_changes(c, &id, &changes)).await {
                            report.errors.push(err(e));
                        }
                    }
                    current.insert(link.id.clone(), states);
                }
                Err(e) => {
                    let msg = format!("{}: {e}", link.scope.name);
                    link_errors.entry(link.id.clone()).or_default().push(msg.clone());
                    report.errors.push(msg);
                    if halts(&e) {
                        halt = Some(e);
                    }
                }
            }
        }

        if halt.is_none() {
            let (flagged, h) = drain_outbox(db, p, &links, &current, link_filter, now, &mut report).await;
            halt = h;
            for link in &targets {
                if halt.is_some() {
                    break;
                }
                let before = report.errors.len();
                halt = pull_link(db, data_dir, p, link, &flagged, now, &mut report).await;
                if (link.auto_import || link.repo_rules.iter().any(|r| r.is_project())) && halt.is_none() {
                    halt = auto_import_link(db, data_dir, p, link, now, &mut report).await;
                }
                link_errors.entry(link.id.clone()).or_default().extend(report.errors[before..].iter().cloned());
            }
        }
        if let Some(e) = &halt {
            let until = now + if e.kind == ErrorKind::Auth { AUTH_PAUSE_MS } else { RATE_LIMIT_PAUSE_MS };
            report.notices.push(format!("{name}: sync paused until {} ({e})", super::iso_from_ms(until)));
            memo.paused.insert(name.to_string(), Pause { until, reason: e.message.clone() });
        }
        for link in &targets {
            let mut errors = link_errors.remove(&link.id).unwrap_or_default();
            if let Some(e) = &halt {
                // Links that never got to sync also show why.
                if errors.is_empty() {
                    errors.push(format!("{}: {e}", link.scope.name));
                }
            }
            let msg = (!errors.is_empty()).then(|| errors.join("\n"));
            let id = link.id.clone();
            if let Err(e) = with_db(db, move |c| store::set_link_sync(c, &id, now, msg.as_deref())).await {
                report.errors.push(err(e));
            }
        }
    }
    report
}

async fn drain_outbox(
    db: &Db,
    p: &Provider,
    links: &[SourceLink],
    current: &HashMap<String, Vec<ExternalState>>,
    link_filter: Option<&str>,
    now: i64,
    report: &mut SyncReport,
) -> (HashSet<String>, Option<ProviderError>) {
    // Tasks the push left an error or notice on: this pass's pull does not clear it.
    let mut flagged = HashSet::new();
    let mut halt = None;
    let name = p.name();
    let items = match with_db(db, move |c| store::due_outbox(c, name, now)).await {
        Ok(i) => i,
        Err(e) => {
            report.errors.push(err(e));
            return (flagged, None);
        }
    };
    // Per-task order: if a row fails, the following rows of that task wait (the closing
    // comment does not go out before the status change that goes with it).
    let mut failed: HashSet<String> = HashSet::new();
    for item in items {
        if failed.contains(&item.task_id) {
            continue;
        }
        let task_id = item.task_id.clone();
        let task = match with_db(db, move |c| rows::get_task(c, &task_id)).await {
            Ok(t) => t,
            Err(e) => {
                report.errors.push(err(e));
                continue;
            }
        };
        let Some((task, src)) = task.and_then(|t| t.source.clone().map(|s| (t, s))) else {
            // Task unlinked in the meantime: the row no longer has a target.
            let id = item.id;
            if let Err(e) = with_db(db, move |c| store::outbox_done(c, id)).await {
                report.errors.push(err(e));
            }
            continue;
        };
        if link_filter.is_some() && src.link_id.as_deref() != link_filter {
            continue;
        }
        let link = src.link_id.as_deref().and_then(|id| links.iter().find(|l| l.id == id));
        let states = link.and_then(|l| current.get(&l.id)).map(Vec::as_slice);
        let action = decide_push(&item.payload, link.map(|l| &l.state_map), states, src.external_state.as_ref());

        let sent: ProviderResult<Option<ExternalState>> = match &action {
            PushAction::Comment(body) => send_comment(p, &src.external_id, body, item.id).await.map(|()| None),
            PushAction::SetState(state_id) => p.set_state(&src.external_id, state_id).await.map(Some),
            PushAction::Drop(_) => Ok(None),
        };
        let (id, task_id) = (item.id, task.id.clone());
        let mut stop = false;
        let result = match (sent, action) {
            (Ok(_), PushAction::Drop(notice)) => {
                if let Some(n) = &notice {
                    report.notices.push(format!("{}: {n}", src.identifier));
                    flagged.insert(task.id.clone());
                }
                with_db(db, move |c| {
                    store::outbox_done(c, id)?;
                    match notice {
                        Some(n) => store::set_sync_error(c, &task_id, Some(&n)),
                        None => Ok(()),
                    }
                })
                .await
            }
            (Ok(state), _) => {
                report.pushed += 1;
                with_db(db, move |c| {
                    store::outbox_done(c, id)?;
                    match state {
                        Some(s) => store::set_external_state(c, &task_id, &s, now),
                        None => store::set_sync_error(c, &task_id, None),
                    }
                })
                .await
            }
            (Err(e), _) => {
                flagged.insert(task.id.clone());
                failed.insert(task.id.clone());
                // With no key or a rate limit, the rest of the rows would fail anyway. Those
                // errors are not the row's: they spend no attempts (a revoked key drops no
                // changes), they only push back the next attempt (the long wait is the
                // provider's pause).
                stop = halts(&e);
                if stop {
                    halt = Some(e.clone());
                }
                let attempts = if stop { item.attempts } else { item.attempts + 1 };
                let give_up = e.kind == ErrorKind::Permanent || (!stop && attempts >= MAX_ATTEMPTS);
                let msg = if give_up {
                    format!("{e} (not retried after {attempts} attempt(s); the change was not pushed)")
                } else {
                    e.message.clone()
                };
                report.errors.push(format!("{}: {msg}", src.identifier));
                let next = now + if stop { BACKOFF_BASE_MS } else { backoff_ms(attempts) };
                with_db(db, move |c| {
                    if give_up {
                        store::outbox_done(c, id)?;
                    } else {
                        store::outbox_retry(c, id, attempts, next, &msg)?;
                    }
                    store::set_sync_error(c, &task_id, Some(&msg))
                })
                .await
            }
        };
        if let Err(e) = result {
            report.errors.push(err(e));
        }
        if stop {
            break;
        }
    }
    (flagged, halt)
}

/// What all tasks of a link share during the pull.
struct PullCtx<'a> {
    data_dir: &'a Path,
    provider: &'a str,
    link: &'a SourceLink,
    now: i64,
}

/// Applies the pull to a task. Returns whether the item existed.
fn apply_pull_task(
    conn: &Connection,
    ctx: &PullCtx,
    task: &Task,
    item: Option<&ExternalItem>,
    keep_error: bool,
) -> Result<bool, DbError> {
    let PullCtx { data_dir, provider, link, now } = *ctx;
    let map = &link.state_map;
    // The task was read before going to the network: if in the meantime it was unlinked,
    // moved to another link or had its plan overwritten, what is there now wins.
    let before = task.source.as_ref().map(|s| (s.link_id.clone(), s.external_id.clone()));
    let Some(task) = rows::get_task(conn, &task.id)?
        .filter(|t| t.source.as_ref().map(|s| (s.link_id.clone(), s.external_id.clone())) == before)
    else {
        return Ok(false);
    };
    let task = &task;
    let Some(item) = item else {
        let msg = format!("Not found in {provider} (deleted or no access).");
        store::set_sync_error(conn, &task.id, Some(&msg))?;
        return Ok(false);
    };
    let active = store::has_active_run(conn, &task.id)?;
    let pending = store::has_pending_state_push(conn, &task.id)?;
    let d = decide_pull(task, item, map, active, pending);
    store::apply_pull(
        conn,
        &store::PullUpdate {
            task_id: &task.id,
            title: d.title.as_deref(),
            status: d.status,
            external_state: d.record_state.then_some(&item.state),
            sync_error: d.sync_error.as_deref(),
            keep_error,
            unmapped: d.unmapped,
            now,
        },
    )?;
    let pu = decide_project(task, item, link);
    store::set_src_project(conn, &task.id, pu.project.as_ref(), pu.rule_id.as_deref(), pu.moved.as_ref())?;
    if !task.plan_overridden && task.plan == PlanRef::Text {
        write_plan(&plan_path(data_dir, &task.id), &render_plan(item))
            .map_err(|e| DbError::Invalid(format!("Could not write the plan of {}: {e}", item.identifier)))?;
    }
    Ok(true)
}

async fn pull_link(
    db: &Db,
    data_dir: &Path,
    p: &Provider,
    link: &SourceLink,
    flagged: &HashSet<String>,
    now: i64,
    report: &mut SyncReport,
) -> Option<ProviderError> {
    let link_id = link.id.clone();
    let tasks = match with_db(db, move |c| store::linked_tasks(c, Some(&link_id), now)).await {
        Ok(t) => t,
        Err(e) => {
            report.errors.push(err(e));
            return None;
        }
    };
    if tasks.is_empty() {
        return None;
    }
    let ids: Vec<String> = tasks.iter().filter_map(|t| t.source.as_ref().map(|s| s.external_id.clone())).collect();
    let items: HashMap<String, ExternalItem> = match p.pull(&ids).await {
        Ok(v) => v.into_iter().map(|i| (i.external_id.clone(), i)).collect(),
        Err(e) => {
            report.errors.push(format!("{}: {e}", link.scope.name));
            return halts(&e).then_some(e);
        }
    };
    let (data_dir, provider, link, flagged) = (data_dir.to_path_buf(), p.name(), link.clone(), flagged.clone());
    let res = with_db(db, move |c| {
        let mut pulled = 0;
        let mut errors = Vec::new();
        let ctx = PullCtx { data_dir: &data_dir, provider, link: &link, now };
        for t in &tasks {
            let item = t.source.as_ref().and_then(|s| items.get(&s.external_id));
            match apply_pull_task(c, &ctx, t, item, flagged.contains(&t.id)) {
                Ok(true) => pulled += 1,
                Ok(false) => {}
                Err(e) => errors.push(e.to_string()),
            }
        }
        Ok((pulled, errors))
    })
    .await;
    match res {
        Ok((n, errors)) => {
            report.pulled += n;
            report.errors.extend(errors);
        }
        Err(e) => report.errors.push(err(e)),
    }
    None
}

async fn auto_import_link(
    db: &Db,
    data_dir: &Path,
    p: &Provider,
    link: &SourceLink,
    now: i64,
    report: &mut SyncReport,
) -> Option<ProviderError> {
    // Queries: the whole scope (items created after the link) if the link has
    // auto-import, and each project rule (items created after the rule; earlier ones
    // are brought in by its backfill). A project rule always auto-imports.
    let mut queries = Vec::new();
    if link.auto_import {
        queries.push(ImportQuery { created_after: Some(link.created_at), ..ImportQuery::new(link.scope.clone()) });
    }
    queries.extend(link.repo_rules.iter().filter(|r| r.is_project()).map(|r| rule_query(link, r, false)));
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    for base in queries {
        let mut cursor = None;
        for _ in 0..AUTO_IMPORT_MAX_PAGES {
            let q = ImportQuery { cursor: cursor.take(), ..base.clone() };
            match p.list_importable(&q).await {
                Ok(page) => {
                    candidates.extend(page.items.into_iter().filter(|i| seen.insert(i.external_id.clone())));
                    match page.next_cursor {
                        Some(c) => cursor = Some(c),
                        None => break,
                    }
                }
                Err(e) => {
                    report.errors.push(format!("{}: {e}", link.scope.name));
                    return halts(&e).then_some(e);
                }
            }
        }
    }
    let l = link.clone();
    let fresh = with_db(db, move |c| {
        let mut out = Vec::new();
        for i in candidates {
            if store::task_by_external(c, &l.provider, &i.external_id)?.is_none()
                && !store::is_unlinked(c, &l.provider, &i.external_id)?
            {
                // A rule or default repo that no longer belongs to the project (deleted) is
                // not retried: the item waits until the source is fixed.
                let repo = suggest_repo_for(&l, &i);
                let usable = match &repo {
                    Some(r) => store::repo_project(c, r)?.as_deref() == Some(l.project_id.as_str()),
                    None => false,
                };
                out.push((repo, usable, i));
            }
        }
        Ok(out)
    })
    .await;
    let fresh = match fresh {
        Ok(f) => f,
        Err(e) => {
            report.errors.push(err(e));
            return None;
        }
    };
    let mut routed: Vec<(String, String)> = Vec::new();
    let mut stale: Vec<String> = Vec::new();
    for (repo, usable, item) in fresh {
        match repo {
            Some(r) if usable => routed.push((item.external_id, r)),
            Some(_) => stale.push(item.identifier),
            None => report.notices.push(format!(
                "{}: not auto-imported (no repo rule matches its labels and the source has no default repo).",
                item.identifier
            )),
        }
    }
    if !stale.is_empty() {
        // Link error (stays in `last_sync_error` while it lasts): not retried every minute.
        report.errors.push(format!(
            "{}: auto-import is waiting for {} ({}): the repo of its rule or the default repo no longer exists or belongs to another project. Review the source's repos.",
            link.scope.name,
            if stale.len() == 1 { "1 item".to_string() } else { format!("{} items", stale.len()) },
            stale.join(", ")
        ));
    }
    if routed.is_empty() {
        return None;
    }
    let ids: Vec<String> = routed.iter().map(|(id, _)| id.clone()).collect();
    let full: HashMap<String, ExternalItem> = match p.pull(&ids).await {
        Ok(v) => v.into_iter().map(|i| (i.external_id.clone(), i)).collect(),
        Err(e) => {
            report.errors.push(format!("{}: {e}", link.scope.name));
            return halts(&e).then_some(e);
        }
    };
    let pairs: Vec<(ExternalItem, String)> =
        routed.into_iter().filter_map(|(id, repo)| full.get(&id).cloned().map(|i| (i, repo))).collect();
    let (link_id, dir) = (link.id.clone(), data_dir.to_path_buf());
    let res = with_db(db, move |c| {
        // `update_source_link` does not take the sync lock: with the link re-read, an item
        // whose routing changed while going to the network waits for the next pass.
        let l = store::get_link(c, &link_id)?;
        let pairs = pairs.into_iter().filter(|(i, repo)| suggest_repo_for(&l, i).as_deref() == Some(repo.as_str())).collect();
        import_items(c, &dir, &l, pairs, now)
    });
    match res.await {
        Ok(r) => {
            report.imported += r.imported.len();
            report.errors.extend(r.skipped.into_iter().map(|s| format!("Auto-import {}: {}", s.external_id, s.reason)));
        }
        Err(e) => report.errors.push(err(e)),
    }
    None
}

// ---------- Worker and command ----------

/// Providers with a key, one pass, under the sync lock. `force`: manual sync (see
/// `sync_run`).
pub async fn run_for_app(app: &AppHandle, link_filter: Option<&str>, force: bool) -> PResult<SyncReport> {
    let db: Db = app.try_state::<Db>().map(|s| s.inner().clone()).ok_or("The database is not available.")?;
    let state = app.state::<ProvidersState>();
    let mut memo = state.sync_lock.lock().await;
    // Pauses are read from `ProvidersState::pauses`: a new key may have lifted them.
    memo.paused = state.paused();
    let before = memo.paused.clone();
    let mut providers = Vec::new();
    for name in super::KNOWN_PROVIDERS {
        match resolve(app, name).await {
            Ok(Some(p)) => providers.push(p),
            Ok(None) => {}
            Err(e) if link_filter.is_some() => return Err(e),
            Err(e) => eprintln!("sync: {name}: {e}"),
        }
    }
    let data_dir: PathBuf = state.data_dir.clone();
    let report = sync_run(&db, &data_dir, &providers, link_filter, now_ms(), &mut memo, force).await;
    // Only what changed in this pass: a new key saved while it ran (which lifted an earlier
    // pause) does not get it back through the copy the pass took.
    state.merge_paused(&before, &memo.paused);
    if !providers.is_empty() {
        use crate::events::Kind;
        // Link state changes on every pass; tasks only if something moved.
        let kinds: &[Kind] =
            if report.pulled + report.pushed + report.imported > 0 { &[Kind::Sources, Kind::Tasks] } else { &[Kind::Sources] };
        crate::events::notify(app, kinds, None);
    }
    Ok(report)
}

pub fn spawn_worker(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_TICK).await;
        loop {
            match run_for_app(&app, None, false).await {
                Ok(r) => {
                    for e in &r.errors {
                        eprintln!("sync: {e}");
                    }
                }
                Err(e) => eprintln!("sync: {e}"),
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

#[cfg(test)]
mod tests;
