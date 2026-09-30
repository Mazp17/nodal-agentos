//! The single run queue: pure logic over the `runs` rows and the `claude agents` output.
//! Ported from the two old queues (issues and tasks), now with a single global
//! concurrency, ordering by `queue_position`, a per-repo lock for `in_place` and migrated
//! runs that await confirmation.

use std::collections::HashSet;

use crate::model::claude::RunSummary;
use crate::model::{Isolation, Run, RunStatus};

/// A just-launched run takes a while to show up in `claude agents`: meanwhile it takes a slot.
pub const LAUNCH_GRACE_MS: i64 = 90_000;
/// A launched run that doesn't show up in `claude agents` after this long is deemed lost.
pub const VANISH_MS: i64 = 10 * 60_000;

/// Run migrated from a previous version that was left queued: it doesn't launch on its
/// own, it has to be confirmed (`confirm_run`) or cancelled.
pub fn awaiting_confirmation(r: &Run) -> bool {
    r.status == RunStatus::Queued && r.legacy_label.is_some()
}

fn live_of<'a>(r: &Run, live: &'a [RunSummary]) -> Option<&'a RunSummary> {
    let id = r.claude_run_id.as_deref()?;
    live.iter().find(|s| s.id == id)
}

/// Is the run still active? Queued/launching, or launched and `working`/`blocked` (or just
/// launched and not yet showing up in `claude agents`).
pub fn is_active(r: &Run, live: Option<&[RunSummary]>, now: i64) -> bool {
    match r.status {
        RunStatus::Queued | RunStatus::Launching => true,
        RunStatus::Finished | RunStatus::Failed | RunStatus::Canceled => false,
        RunStatus::Launched => {
            let Some(live) = live else { return true };
            match live_of(r, live) {
                Some(s) => s.is_in_progress(),
                None => r.launched_at.is_some_and(|t| now - t < LAUNCH_GRACE_MS),
            }
        }
    }
}

/// Occupied slots: our own runs that are launching or launched and still in progress
/// (`working`, `blocked` waiting on the user, or launched recently and not listed yet).
/// Sessions Nodal didn't launch (another Nodal, a manual `claude --bg`) take no slot, and
/// ended runs never do.
pub fn occupied_slots(runs: &[Run], live: &[RunSummary], now: i64) -> usize {
    runs.iter()
        .filter(|r| matches!(r.status, RunStatus::Launching | RunStatus::Launched))
        .filter(|r| is_active(r, Some(live), now))
        .count()
}

/// Queue summary for the UI.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkSummary {
    /// Occupied slots (`occupied_slots` rule).
    pub running: u32,
    /// Global concurrency from Settings.
    pub capacity: u32,
    /// Distinct things waiting on the user: tasks Blocked or moved to another project in the
    /// provider, unconfirmed migrated runs and sessions waiting for permission/input. A task
    /// counts only once.
    pub need_you: u32,
    /// Queued and ready to go (excluding those awaiting confirmation).
    pub queued: u32,
    /// Error of the last queue pass (e.g. `claude agents` fails), or `null`.
    pub pump_error: Option<String>,
}

/// The session is waiting on the user (permission, input, dialog).
fn waiting(s: &RunSummary) -> bool {
    s.state.as_deref() == Some("blocked") || s.status.as_deref() == Some("waiting")
}

/// `runs`: the pending ones in scope (`pending`/`pending_of`); `blocked_tasks`: ids of the
/// tasks in scope waiting on the user. `global`: no project filter, so foreign sessions
/// waiting on the user count in `need_you` too. `running` is always our own runs that take
/// a slot.
pub fn work_summary(
    runs: &[Run],
    blocked_tasks: &[String],
    live: &[RunSummary],
    concurrency: u32,
    global: bool,
    now: i64,
) -> WorkSummary {
    let running = occupied_slots(runs, live, now);
    let queued = runs
        .iter()
        .filter(|r| r.status == RunStatus::Queued && !awaiting_confirmation(r))
        .count();

    let key = |r: &Run| match &r.task_id {
        Some(t) => format!("task:{t}"),
        None => format!("run:{}", r.id),
    };
    let mut need: HashSet<String> = blocked_tasks.iter().map(|t| format!("task:{t}")).collect();
    let mut own_sessions: HashSet<&str> = HashSet::new();
    for r in runs {
        if awaiting_confirmation(r) {
            need.insert(key(r));
        }
        if let Some(s) = (r.status == RunStatus::Launched)
            .then(|| live_of(r, live))
            .flatten()
        {
            own_sessions.insert(s.id.as_str());
            if waiting(s) {
                need.insert(key(r));
            }
        }
    }
    if global {
        for s in live
            .iter()
            .filter(|s| waiting(s) && !own_sessions.contains(s.id.as_str()))
        {
            need.insert(format!("session:{}", s.session_id));
        }
    }
    WorkSummary {
        running: running as u32,
        capacity: concurrency,
        need_you: need.len() as u32,
        queued: queued as u32,
        pump_error: None,
    }
}

/// Repos with an active `in_place` run: the queue doesn't launch another one there.
fn locked_repos(runs: &[Run], live: &[RunSummary], now: i64) -> HashSet<String> {
    runs.iter()
        .filter(|r| r.isolation == Some(Isolation::InPlace))
        .filter(|r| matches!(r.status, RunStatus::Launching | RunStatus::Launched))
        .filter(|r| is_active(r, Some(live), now))
        .filter_map(|r| r.repo_id.clone())
        .collect()
}

/// Ids of the queued runs to launch now, in order (`queue_position`, `queued_at`), based on
/// the free slots. Skips those awaiting confirmation and the `in_place` ones whose repo is
/// busy (without holding back the ones behind).
pub fn next_to_launch(
    runs: &[Run],
    live: &[RunSummary],
    concurrency: u32,
    now: i64,
) -> Vec<String> {
    let mut free = (concurrency as usize).saturating_sub(occupied_slots(runs, live, now));
    let mut locked = locked_repos(runs, live, now);
    let mut queued: Vec<&Run> = runs
        .iter()
        .filter(|r| r.status == RunStatus::Queued && !awaiting_confirmation(r))
        .collect();
    queued.sort_by(|a, b| {
        a.queue_position
            .total_cmp(&b.queue_position)
            .then(a.queued_at.cmp(&b.queued_at))
            .then(a.id.cmp(&b.id))
    });
    let mut out = Vec::new();
    for r in queued {
        if free == 0 {
            break;
        }
        if r.isolation == Some(Isolation::InPlace) {
            if let Some(repo) = &r.repo_id {
                if !locked.insert(repo.clone()) {
                    continue;
                }
            }
        }
        out.push(r.id.clone());
        free -= 1;
    }
    out
}

/// Fills in `session_id` by matching `claude_run_id` against `claude agents`. Returns the
/// indices that changed.
pub fn fill_session_ids(runs: &mut [Run], live: &[RunSummary]) -> Vec<usize> {
    let mut changed = Vec::new();
    for (i, r) in runs
        .iter_mut()
        .enumerate()
        .filter(|(_, r)| r.session_id.is_none())
    {
        if let Some(s) = live_of(r, live) {
            r.session_id = Some(s.session_id.clone());
            changed.push(i);
        }
    }
    changed
}

/// Clock slack between `claude agents`' `started_at` and the moment of the launch.
pub const ADOPT_SLACK_MS: i64 = 5_000;

/// The session probably started by a `claude --bg` that didn't return its id in time: the
/// only one in `cwd`, started since `since` and not held by any run (`claimed`: their
/// `claude_run_id`s). With more than one candidate, no guessing.
pub fn adoptable<'a>(
    cwd: &str,
    since: i64,
    live: &'a [RunSummary],
    claimed: &[String],
) -> Option<&'a RunSummary> {
    let norm = |p: &str| p.trim_end_matches('/').to_string();
    let want = norm(cwd);
    let mut found = live.iter().filter(|s| {
        s.cwd.as_deref().map(norm).as_deref() == Some(want.as_str())
            && s.started_at.is_some_and(|t| t >= since - ADOPT_SLACK_MS)
            && !claimed.contains(&s.id)
    });
    let first = found.next()?;
    found.next().is_none().then_some(first)
}

/// Is there anything that requires querying `claude agents`? Queued runs that can launch,
/// or launched ones (to detect when they finish).
pub fn needs_tick(runs: &[Run]) -> bool {
    runs.iter().any(|r| {
        (r.status == RunStatus::Queued && !awaiting_confirmation(r))
            || r.status == RunStatus::Launched
    })
}

/// How a session ended, according to `claude agents`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndSignal {
    /// `state: "done"`: it finished its turn.
    Done,
    /// `state: "failed"`.
    Failed,
    /// `state: "stopped"` (`claude stop` or it was closed).
    Stopped,
    /// Not listed in `claude agents` for more than `VANISH_MS`.
    Vanished,
}

/// Launched runs that are no longer in progress (left working/blocked).
pub fn ended(runs: &[Run], live: &[RunSummary], now: i64) -> Vec<(String, EndSignal)> {
    runs.iter()
        .filter(|r| r.status == RunStatus::Launched)
        .filter_map(|r| {
            let signal = match live_of(r, live) {
                Some(s) if s.is_in_progress() => return None,
                Some(s) => match s.state.as_deref() {
                    Some("done") => EndSignal::Done,
                    Some("failed") => EndSignal::Failed,
                    Some("stopped") => EndSignal::Stopped,
                    // Unknown state: wait (it may be a new "in progress" one).
                    _ => return None,
                },
                None if r.launched_at.is_some_and(|t| now - t >= VANISH_MS) => EndSignal::Vanished,
                None => return None,
            };
            Some((r.id.clone(), signal))
        })
        .collect()
}

#[cfg(test)]
mod adopt_tests;

#[cfg(test)]
mod tests;
