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
