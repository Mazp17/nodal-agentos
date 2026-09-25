//! The single run queue: pure logic over the `runs` rows and the `claude agents` output.
//! Ported from the two old queues (issues and tasks), now with a single global
//! concurrency, ordering by `queue_position`, a per-repo lock for `in_place` and migrated
//! runs that await confirmation.

use std::collections::HashSet;

use crate::domain::{Isolation, Run, RunStatus};
use crate::runs::types::RunSummary;

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

/// Occupied slots: every `working` background session (ours or not), plus ours that are
/// launching or were launched recently and aren't listed yet.
/// `blocked` (waiting for permission/input) doesn't take a slot: if nobody answers it would
/// stall the queue.
pub fn occupied_slots(runs: &[Run], live: &[RunSummary], now: i64) -> usize {
    let working = live.iter().filter(|s| s.state.as_deref() == Some("working")).count();
    let pending = runs
        .iter()
        .filter(|r| match r.status {
            RunStatus::Launching => true,
            RunStatus::Launched => {
                live_of(r, live).is_none() && r.launched_at.is_some_and(|t| now - t < LAUNCH_GRACE_MS)
            }
            _ => false,
        })
        .count();
    working + pending
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
/// Blocked tasks in scope. `global`: no project filter, so foreign sessions count too
/// (working ones take a slot; waiting ones need the user). With a project, `running` is
/// only our own runs that take a slot.
pub fn work_summary(
    runs: &[Run],
    blocked_tasks: &[String],
    live: &[RunSummary],
    concurrency: u32,
    global: bool,
    now: i64,
) -> WorkSummary {
    let running = if global {
        occupied_slots(runs, live, now)
    } else {
        runs.iter()
            .filter(|r| match r.status {
                RunStatus::Launching => true,
                RunStatus::Launched => match live_of(r, live) {
                    Some(s) => s.state.as_deref() == Some("working"),
                    None => r.launched_at.is_some_and(|t| now - t < LAUNCH_GRACE_MS),
                },
                _ => false,
            })
            .count()
    };
    let queued = runs.iter().filter(|r| r.status == RunStatus::Queued && !awaiting_confirmation(r)).count();

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
        if let Some(s) = (r.status == RunStatus::Launched).then(|| live_of(r, live)).flatten() {
            own_sessions.insert(s.id.as_str());
            if waiting(s) {
                need.insert(key(r));
            }
        }
    }
    if global {
        for s in live.iter().filter(|s| waiting(s) && !own_sessions.contains(s.id.as_str())) {
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
pub fn next_to_launch(runs: &[Run], live: &[RunSummary], concurrency: u32, now: i64) -> Vec<String> {
    let mut free = (concurrency as usize).saturating_sub(occupied_slots(runs, live, now));
    let mut locked = locked_repos(runs, live, now);
    let mut queued: Vec<&Run> =
        runs.iter().filter(|r| r.status == RunStatus::Queued && !awaiting_confirmation(r)).collect();
    queued.sort_by(|a, b| {
        a.queue_position.total_cmp(&b.queue_position).then(a.queued_at.cmp(&b.queued_at)).then(a.id.cmp(&b.id))
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
    for (i, r) in runs.iter_mut().enumerate().filter(|(_, r)| r.session_id.is_none()) {
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
pub fn adoptable<'a>(cwd: &str, since: i64, live: &'a [RunSummary], claimed: &[String]) -> Option<&'a RunSummary> {
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
    runs.iter()
        .any(|r| (r.status == RunStatus::Queued && !awaiting_confirmation(r)) || r.status == RunStatus::Launched)
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
mod adopt_tests {
    use super::*;

    fn session(id: &str, cwd: &str, started: i64) -> RunSummary {
        RunSummary {
            id: id.into(),
            session_id: format!("sess-{id}"),
            cwd: Some(cwd.into()),
            name: None,
            started_at: Some(started),
            pid: None,
            status: None,
            state: Some("working".into()),
            waiting_for: None,
        }
    }

    #[test]
    fn adopts_only_an_unambiguous_new_session_in_the_cwd() {
        let live = vec![session("old", "/r/web", 100), session("other", "/r/api", 10_000), session("new", "/r/web/", 10_000)];
        assert_eq!(adoptable("/r/web", 9_000, &live, &[]).map(|s| s.id.as_str()), Some("new"));
        // Another run already has it.
        assert!(adoptable("/r/web", 9_000, &live, &["new".into()]).is_none());
        // Two candidates: no guessing.
        let mut two = live.clone();
        two.push(session("new2", "/r/web", 11_000));
        assert!(adoptable("/r/web", 9_000, &two, &[]).is_none());
        // Before the launch (outside the slack).
        assert!(adoptable("/r/web", 20_000, &live, &[]).is_none());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Executor, Finish, LaunchOptions, RunKind};

    const NOW: i64 = 10_000_000;

    pub(crate) fn run(id: &str, status: RunStatus, pos: f64) -> Run {
        Run {
            id: id.into(),
            task_id: Some(format!("task-{id}")),
            repo_id: Some("repo".into()),
            cwd: "/repo".into(),
            executor: Executor::Claude,
            kind: RunKind::Work,
            parent_run_id: None,
            prompt: "x".into(),
            extra_instructions: None,
            options: LaunchOptions::default(),
            finish: Finish::Pr,
            isolation: Some(Isolation::Worktree),
            review: false,
            verdict: None,
            status,
            queue_position: pos,
            claude_run_id: None,
            session_id: None,
            queued_at: pos as i64,
            launched_at: None,
            finished_at: None,
            outcome: None,
            summary: None,
            pr_url: None,
            branch: None,
            error: None,
            legacy_label: None,
            tokens: None,
        }
    }

    fn launched(id: &str, claude_id: &str, at: i64) -> Run {
        Run { claude_run_id: Some(claude_id.into()), launched_at: Some(at), ..run(id, RunStatus::Launched, 0.0) }
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

    #[test]
    fn work_summary_counts_slots_and_dedupes_need_you() {
        let mut waiting_run = launched("a", "aaaa", NOW - 1_000_000);
        waiting_run.task_id = Some("t-blocked".into());
        let mut legacy = run("l", RunStatus::Queued, 2.0);
        legacy.legacy_label = Some("ENG-1".into());
        let runs = vec![
            waiting_run,
            launched("b", "bbbb", NOW - 1_000_000),
            launched("c", "cccc", NOW - 1_000),
            run("q", RunStatus::Queued, 1.0),
            legacy,
            run("n", RunStatus::Launching, 3.0),
        ];
        let mut perm = live("aaaa", "blocked");
        perm.status = Some("waiting".into());
        let mut foreign_wait = live("ffff", "working");
        foreign_wait.status = Some("waiting".into());
        let lv = vec![perm, live("bbbb", "working"), live("xxxx", "working"), foreign_wait];
        let blocked = vec!["t-blocked".to_string(), "t-other".to_string()];

        let g = work_summary(&runs, &blocked, &lv, 3, true, NOW);
        // working: bbbb, xxxx, ffff (foreign included) + c (in grace) + n (launching).
        assert_eq!((g.running, g.capacity, g.queued), (5, 3, 1));
        // t-blocked (Blocked task and its session waiting for permission: once), t-other, the
        // migrated one and the waiting foreign session.
        assert_eq!(g.need_you, 4, "{g:?}");

        let p = work_summary(&runs, &blocked, &lv, 3, false, NOW);
        // Own only: b (working), c (grace), n (launching). `a` is blocked: takes no slot.
        assert_eq!((p.running, p.queued, p.need_you), (3, 1, 3), "{p:?}");
        assert_eq!(work_summary(&[], &[], &[], 2, false, NOW), WorkSummary { capacity: 2, ..Default::default() });
        let v = serde_json::to_value(WorkSummary { pump_error: Some("`claude agents` failed".into()), ..Default::default() }).unwrap();
        assert_eq!(v["pumpError"], "`claude agents` failed");
        assert!(serde_json::to_value(WorkSummary::default()).unwrap()["pumpError"].is_null());
    }

    #[test]
    fn queue_follows_position_and_respects_free_slots() {
        let runs = vec![
            run("c", RunStatus::Queued, 30.0),
            run("a", RunStatus::Queued, 10.0),
            run("b", RunStatus::Queued, 20.0),
            run("x", RunStatus::Failed, 1.0),
        ];
        let lv = vec![live("aa11", "working"), live("bb22", "done"), live("cc33", "stopped")];
        // 3 slots, 1 foreign working → 2 free: the first two in the queue.
        assert_eq!(next_to_launch(&runs, &lv, 3, NOW), ["a", "b"]);
        assert_eq!(next_to_launch(&runs, &lv, 1, NOW), Vec::<String>::new());
        assert_eq!(next_to_launch(&runs, &[], 10, NOW), ["a", "b", "c"]);
        // Reordering changes who goes next.
        let mut reordered = runs.clone();
        reordered[0].queue_position = 1.0;
        assert_eq!(next_to_launch(&reordered, &lv, 3, NOW), ["c", "a"]);
    }

    #[test]
    fn launching_and_fresh_launches_occupy_slots() {
        let runs = vec![
            run("q", RunStatus::Queued, 5.0),
            run("l", RunStatus::Launching, 1.0),
            // Just launched, not yet listed in claude agents: takes a slot.
            launched("f", "ffff0001", NOW - 1_000),
            // Launched long ago and not listed: takes no slot.
            launched("o", "ffff0002", NOW - LAUNCH_GRACE_MS - 1),
            // Listed and finished: takes no slot (nor double counts).
            launched("d", "dddd0001", NOW - 1_000),
        ];
        let lv = vec![live("dddd0001", "done")];
        assert_eq!(occupied_slots(&runs, &lv, NOW), 2);
        assert!(next_to_launch(&runs, &lv, 2, NOW).is_empty());
        assert_eq!(next_to_launch(&runs, &lv, 3, NOW), ["q"]);
        // If listed and working, it counts only once (via claude agents).
        let lv = vec![live("ffff0001", "working"), live("dddd0001", "done")];
        assert_eq!(occupied_slots(&runs, &lv, NOW), 2);
    }

    #[test]
    fn global_concurrency_never_exceeded() {
        // Four queued with concurrency 2: 2 go, and with those 2 working no more go.
        let mut runs: Vec<Run> = (0..4).map(|i| run(&format!("r{i}"), RunStatus::Queued, i as f64)).collect();
        let first = next_to_launch(&runs, &[], 2, NOW);
        assert_eq!(first, ["r0", "r1"]);
        for (i, id) in first.iter().enumerate() {
            let r = runs.iter_mut().find(|r| &r.id == id).unwrap();
            r.status = RunStatus::Launched;
            r.claude_run_id = Some(format!("c{i}00000"));
            r.launched_at = Some(NOW);
        }
        let lv = vec![live("c000000", "working"), live("c100000", "working")];
        assert!(next_to_launch(&runs, &lv, 2, NOW).is_empty());
        let lv = vec![live("c000000", "done"), live("c100000", "working")];
        assert_eq!(next_to_launch(&runs, &lv, 2, NOW), ["r2"]);
    }

    #[test]
    fn in_place_runs_lock_their_repo() {
        let mut a = run("a", RunStatus::Queued, 1.0);
        a.isolation = Some(Isolation::InPlace);
        let mut b = run("b", RunStatus::Queued, 2.0);
        b.isolation = Some(Isolation::InPlace);
        let mut c = run("c", RunStatus::Queued, 3.0);
        c.isolation = Some(Isolation::InPlace);
        c.repo_id = Some("other".into());
        let d = run("d", RunStatus::Queued, 4.0);
        // Two in_place on the same repo: one goes; the other repo's and the worktree one proceed.
        assert_eq!(next_to_launch(&[a.clone(), b.clone(), c.clone(), d.clone()], &[], 10, NOW), ["a", "c", "d"]);
        // With an active in_place run in the repo, the next one waits.
        let mut active = launched("x", "xxxx0001", NOW - 1_000);
        active.isolation = Some(Isolation::InPlace);
        let lv = vec![live("xxxx0001", "working")];
        assert_eq!(next_to_launch(&[active.clone(), b.clone(), d.clone()], &lv, 10, NOW), ["d"]);
        // When it finishes, it's released.
        let lv = vec![live("xxxx0001", "done")];
        assert_eq!(next_to_launch(&[active, b, d], &lv, 10, NOW), ["b", "d"]);
    }

    #[test]
    fn migrated_queued_runs_wait_for_confirmation() {
        let mut legacy = run("old", RunStatus::Queued, 0.5);
        legacy.legacy_label = Some("ENG-1".into());
        let runs = vec![legacy.clone(), run("new", RunStatus::Queued, 1.0)];
        assert!(awaiting_confirmation(&legacy));
        assert_eq!(next_to_launch(&runs, &[], 5, NOW), ["new"]);
        assert!(!needs_tick(&[legacy]));
    }

    #[test]
    fn session_ids_are_filled_from_live_runs() {
        let mut runs = vec![launched("a", "aaaa0001", NOW), run("q", RunStatus::Queued, 1.0)];
        assert!(needs_tick(&runs));
        assert!(fill_session_ids(&mut runs, &[live("zzzz", "working")]).is_empty());
        assert_eq!(fill_session_ids(&mut runs, &[live("aaaa0001", "working")]), [0]);
        assert_eq!(runs[0].session_id.as_deref(), Some("sess-aaaa0001"));
        let finished = vec![run("f", RunStatus::Finished, 1.0), run("x", RunStatus::Canceled, 2.0)];
        assert!(!needs_tick(&finished));
    }

    #[test]
    fn active_detection() {
        let lv = vec![live("aaaa", "working"), live("bbbb", "done"), live("cccc", "blocked")];
        assert!(is_active(&run("q", RunStatus::Queued, 1.0), None, NOW));
        assert!(!is_active(&run("f", RunStatus::Failed, 1.0), Some(&lv), NOW));
        assert!(is_active(&launched("a", "aaaa", 1), Some(&lv), NOW));
        assert!(!is_active(&launched("b", "bbbb", NOW), Some(&lv), NOW));
        // `blocked` is still active (not relaunched) but takes no slot.
        assert!(is_active(&launched("c", "cccc", 1), Some(&lv), NOW));
        assert_eq!(occupied_slots(&[], &[live("cccc", "blocked")], NOW), 0);
        assert!(is_active(&launched("d", "dddd", NOW - 10), Some(&lv), NOW));
        assert!(!is_active(&launched("d", "dddd", NOW - LAUNCH_GRACE_MS), Some(&lv), NOW));
        // Without `claude agents` it's assumed active (no blind relaunch).
        assert!(is_active(&launched("e", "eeee", 1), None, NOW));
    }

    #[test]
    fn end_detection() {
        let runs = vec![
            launched("w", "wwww", NOW - 1_000),
            launched("d", "dddd", NOW - 1_000),
            launched("s", "ssss", NOW - 1_000),
            launched("f", "ffff", NOW - 1_000),
            launched("b", "bbbb", NOW - 1_000),
            launched("fresh", "nnnn", NOW - 1_000),
            launched("gone", "gggg", NOW - VANISH_MS),
            run("q", RunStatus::Queued, 1.0),
        ];
        let lv = vec![
            live("wwww", "working"),
            live("dddd", "done"),
            live("ssss", "stopped"),
            live("ffff", "failed"),
            live("bbbb", "blocked"),
        ];
        let ends = ended(&runs, &lv, NOW);
        assert_eq!(
            ends,
            vec![
                ("d".to_string(), EndSignal::Done),
                ("s".to_string(), EndSignal::Stopped),
                ("f".to_string(), EndSignal::Failed),
                ("gone".to_string(), EndSignal::Vanished),
            ]
        );
    }
}
