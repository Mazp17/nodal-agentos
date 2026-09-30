use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::*;
use crate::claude::activity_files;
use crate::testutil::block_on;
use nodal_domain::model::activity::AgentSession;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/claude/fixtures/activity")
}

const COORD: &str = "11111111-2222-3333-4444-555555555555";
const BG: &str = "99999999-0000-0000-0000-000000000001";

/// The time windows depend on mtime: the fixtures are copied to a temp dir
/// (mtime = now) and the ones that must look old get their mtime adjusted.
fn setup(name: &str) -> (PathBuf, i64) {
    let dst = std::env::temp_dir().join(format!("nodal-activity-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dst);
    copy_dir(&fixtures().join("projects"), &dst);
    let old = std::time::SystemTime::now() - Duration::from_secs(3 * 3600);
    let rel =
        "-Users-me-Code/11111111-2222-3333-4444-555555555555/subagents/agent-aold00000000001.jsonl";
    let f = std::fs::File::options()
        .append(true)
        .open(dst.join(rel))
        .unwrap();
    f.set_modified(old).unwrap();
    let now = activity_files::mtime_ms(&dst.join("-Users-me-Code-repo").join(format!("{BG}.jsonl")))
        .unwrap()
        + 1_000;
    (dst, now)
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap().flatten() {
        let to = dst.join(e.file_name());
        if e.path().is_dir() {
            copy_dir(&e.path(), &to);
        } else {
            std::fs::copy(e.path(), &to).unwrap();
            // On macOS `fs::copy` keeps the source mtime (the checkout's): set it to now so
            // the time windows don't depend on when the repo was cloned.
            let f = std::fs::File::options().append(true).open(&to).unwrap();
            f.set_modified(std::time::SystemTime::now()).unwrap();
        }
    }
}

/// `startedAt` relative to `now`: the repo's finished one is from 1 h ago (recent).
fn agents(now: i64) -> Vec<AgentSession> {
    let mut a = activity_files::parse_agents(
        &std::fs::read_to_string(fixtures().join("agents.json")).unwrap(),
    )
    .unwrap();
    for s in a.iter_mut() {
        s.started_at = Some((now - 3_600_000) as f64);
    }
    a
}

#[test]
fn repo_activity_from_fixtures() {
    let (projects, now) = setup("main");
    let mut app = AppRuns::default();
    app.add(Some("99999999".into()), None);
    let act = assemble(
        &[PathBuf::from("/Users/me/Code/repo")],
        &agents(now),
        &projects,
        &app,
        now,
    );

    let ids: Vec<(&str, &str, bool)> = act
        .sessions
        .iter()
        .map(|s| (s.session_id.as_str(), s.kind.as_str(), s.alive))
        .collect();
    // The repo's working background session, an interactive one in a repo worktree, an
    // unlisted headless one, a recently finished background one. Nothing from the sandbox
    // or the coordinator.
    assert!(ids.contains(&(BG, "background", true)), "{ids:?}");
    assert!(
        ids.contains(&("77777777-0000-0000-0000-000000000003", "interactive", true)),
        "{ids:?}"
    );
    assert!(
        ids.contains(&("88888888-aaaa-bbbb-cccc-000000000001", "unlisted", true)),
        "{ids:?}"
    );
    assert!(
        ids.contains(&("43495c2e-056b-4f96-8cab-4dd89a61e005", "background", false)),
        "{ids:?}"
    );
    assert_eq!(ids.len(), 4, "{ids:?}");
    assert!(!ids[3].2, "live ones first");

    let bg = act.sessions.iter().find(|s| s.session_id == BG).unwrap();
    assert!(bg.is_app_run);
    assert_eq!(bg.last_tool.as_deref(), Some("Read"));
    let unlisted = act.sessions.iter().find(|s| s.kind == "unlisted").unwrap();
    assert_eq!(unlisted.entrypoint.as_deref(), Some("sdk-cli"));
    assert!(!unlisted.is_app_run);

    let subs: Vec<(&str, bool)> = act
        .subagents
        .iter()
        .map(|s| (s.agent_id.as_str(), s.active))
        .collect();
    // From the coordinator (cwd outside the repo): the worktree one, its child with an
    // inherited worktree and the one that cd'd into the repo. Not the finished one outside
    // the repo, nor the old one.
    assert!(subs.contains(&("alive0000000001", true)), "{subs:?}");
    assert!(subs.contains(&("achild000000001", false)), "{subs:?}");
    assert!(subs.contains(&("acdrepo00000001", true)), "{subs:?}");
    // Workflow agent of the repo's background session.
    assert!(subs.contains(&("aworkflow000000001", true)), "{subs:?}");
    assert_eq!(subs.len(), 4, "{subs:?}");
    assert!(
        act.subagents[..3].iter().all(|s| s.active),
        "active ones first"
    );

    let alive = act
        .subagents
        .iter()
        .find(|s| s.agent_id == "alive0000000001")
        .unwrap();
    assert_eq!(alive.session_id, COORD);
    assert_eq!(alive.session_kind, "interactive");
    assert_eq!(
        alive.worktree.as_deref(),
        Some("/Users/me/Code/repo/.claude/worktrees/agent-alive0000000001")
    );
    assert_eq!(alive.description.as_deref(), Some("Local tasks backend"));
    assert_eq!(alive.last_tool.as_deref(), Some("Bash"));
    let child = act
        .subagents
        .iter()
        .find(|s| s.agent_id == "achild000000001")
        .unwrap();
    assert!(child.finished);
    assert_eq!(child.parent_agent_id.as_deref(), Some("alive0000000001"));
    let wf = act
        .subagents
        .iter()
        .find(|s| s.agent_id == "aworkflow000000001")
        .unwrap();
    assert_eq!(wf.workflow_id.as_deref(), Some("wf_1234"));
    assert!(wf.session_is_app_run);

    std::fs::remove_dir_all(&projects).unwrap();
}

#[test]
fn external_sessions_skip_app_runs_and_land_in_the_deepest_repo() {
    let (projects, now) = setup("external");
    let id = |v: Vec<PathBuf>| v;
    let mut app = AppRuns::default();
    app.add(Some("99999999".into()), None);
    let repo = PathBuf::from("/Users/me/Code/repo");
    let repos = vec![
        ("r1".to_string(), repo.clone()),
        ("r2".to_string(), repo.join(".claude/worktrees/feature-x")),
        (
            "r3".to_string(),
            repo.join(".claude/worktrees/agent-alive0000000001"),
        ),
        (
            "r4".to_string(),
            PathBuf::from("/Users/me/Code/repo-sandbox"),
        ),
        ("r5".to_string(), PathBuf::from("/Users/me/Code/elsewhere")),
    ];
    let ext = external_sessions_of(&repos, &agents(now), &projects, &app, now, id);
    assert_eq!(ext.generated_at, now);
    let find = |rid: &str| ext.repos.iter().find(|r| r.repo_id == rid);
    let sessions = |rid: &str| {
        let mut v: Vec<&str> = find(rid).map_or(vec![], |r| {
            r.sessions.iter().map(|s| s.session_id.as_str()).collect()
        });
        v.sort();
        v
    };
    let subagents = |rid: &str| {
        let mut v: Vec<&str> = find(rid).map_or(vec![], |r| {
            r.subagents.iter().map(|s| s.agent_id.as_str()).collect()
        });
        v.sort();
        v
    };

    // The app's background run (and its workflow agent) is left out; the worktree
    // session goes to the nested repo, not to the outer one.
    assert_eq!(
        sessions("r1"),
        vec![
            "43495c2e-056b-4f96-8cab-4dd89a61e005",
            "88888888-aaaa-bbbb-cccc-000000000001"
        ]
    );
    assert_eq!(sessions("r2"), vec!["77777777-0000-0000-0000-000000000003"]);
    assert!(
        sessions("r4").contains(&"af5deb85-3fe1-4ea9-b172-00b68389e167"),
        "{ext:?}"
    );
    // Subagents in a nested repo's worktree go there; the one that cd'd into the repo stays in r1.
    assert_eq!(subagents("r1"), vec!["acdrepo00000001"]);
    assert_eq!(subagents("r3"), vec!["achild000000001", "alive0000000001"]);
    assert!(ext
        .repos
        .iter()
        .all(|r| r.sessions.iter().all(|s| !s.is_app_run)));
    assert!(ext
        .repos
        .iter()
        .all(|r| r.subagents.iter().all(|s| !s.session_is_app_run)));
    // Repos with nothing to show are omitted.
    assert!(find("r5").is_none(), "{ext:?}");

    // Each session and subagent appears exactly once.
    let all_sessions: Vec<&str> = ext
        .repos
        .iter()
        .flat_map(|r| r.sessions.iter().map(|s| s.session_id.as_str()))
        .collect();
    assert_eq!(
        all_sessions.len(),
        all_sessions.iter().collect::<HashSet<_>>().len(),
        "{all_sessions:?}"
    );
    let all_subs: Vec<(&str, &str)> = ext
        .repos
        .iter()
        .flat_map(|r| {
            r.subagents
                .iter()
                .map(|s| (s.session_id.as_str(), s.agent_id.as_str()))
        })
        .collect();
    assert_eq!(
        all_subs.len(),
        all_subs.iter().collect::<HashSet<_>>().len(),
        "{all_subs:?}"
    );

    // In the app, a missing folder yields nothing; no repos, nothing.
    assert!(
        external_sessions_of(&repos, &agents(now), &projects, &app, now, existing_roots)
            .repos
            .is_empty()
    );
    assert!(
        external_sessions_of(&[], &agents(now), &projects, &app, now, id)
            .repos
            .is_empty()
    );
    std::fs::remove_dir_all(&projects).unwrap();
}

/// P07: with several repos whose roots overlap (nested worktrees), a session or subagent
/// transcript relevant to more than one of them must be read once total, not once per repo
/// that matches it — the property `external_sessions_of`'s single `assemble_all` pass relies
/// on instead of looping `assemble` per repo.
#[test]
fn external_sessions_reads_each_transcript_once_across_repos() {
    let (projects, now) = setup("external-once");
    let mut app = AppRuns::default();
    app.add(Some("99999999".into()), None);
    let repo = PathBuf::from("/Users/me/Code/repo");
    let repo_paths = vec![
        repo.clone(),
        repo.join(".claude/worktrees/feature-x"),
        repo.join(".claude/worktrees/agent-alive0000000001"),
        PathBuf::from("/Users/me/Code/repo-sandbox"),
        PathBuf::from("/Users/me/Code/elsewhere"),
    ];
    let agents = agents(now);

    // A repo-at-a-time baseline: one `assemble` call per repo, like `external_sessions_of`
    // did before P07.
    activity_files::READ_TAIL_CALLS.with(|c| c.set(0));
    for p in &repo_paths {
        let _ = assemble(std::slice::from_ref(p), &agents, &projects, &app, now);
    }
    let per_repo_reads = activity_files::READ_TAIL_CALLS.with(|c| c.get());

    // The shared pass every repo goes through together.
    activity_files::READ_TAIL_CALLS.with(|c| c.set(0));
    let repo_roots: Vec<Vec<PathBuf>> = repo_paths.iter().map(|p| vec![p.clone()]).collect();
    let _ = assemble_all(&repo_roots, &agents, &projects, &app, now);
    let shared_reads = activity_files::READ_TAIL_CALLS.with(|c| c.get());

    assert!(
        shared_reads < per_repo_reads,
        "shared pass read {shared_reads} transcripts, one `assemble` per repo read {per_repo_reads}"
    );

    std::fs::remove_dir_all(&projects).unwrap();
}

#[test]
fn match_depth_picks_the_deepest_containing_root() {
    let roots = vec![PathBuf::from("/a"), PathBuf::from("/a/b")];
    assert_eq!(match_depth(&roots, [Some("/a/b/c")]), 3);
    assert_eq!(match_depth(&roots, [Some("/a/x"), None]), 2);
    assert_eq!(match_depth(&roots, [Some("/z"), Some("rel/a")]), 0);
}

#[test]
fn dead_session_has_no_active_subagents_and_stale_ones_expire() {
    let (projects, now) = setup("dead");
    let mut agents = agents(now);
    // The coordinator died: its subagents remain, but inactive.
    let coord = agents
        .iter_mut()
        .find(|a| a.session_id.as_deref() == Some(COORD))
        .unwrap();
    coord.pid = None;
    coord.status = None;
    let act = assemble(
        &[PathBuf::from("/Users/me/Code/repo")],
        &agents,
        &projects,
        &AppRuns::default(),
        now,
    );
    assert!(
        act.subagents.iter().all(|s| s.session_id != COORD),
        "dead session outside the repo: not scanned"
    );

    // After 10 min without writing, a subagent without `end_turn` stops counting as active.
    let act = assemble(
        &[PathBuf::from("/Users/me/Code/repo")],
        &self::agents(now),
        &projects,
        &AppRuns::default(),
        now + STALE_MS + 1,
    );
    assert!(
        act.subagents.iter().all(|s| !s.active),
        "{:?}",
        act.subagents
    );
    std::fs::remove_dir_all(&projects).unwrap();
}

#[test]
fn repo_prefix_is_not_containment() {
    let roots = RepoRoots(vec![PathBuf::from("/Users/me/Code/repo")]);
    assert!(roots.contains("/Users/me/Code/repo"));
    assert!(roots.contains("/Users/me/Code/repo/.claude/worktrees/x"));
    assert!(!roots.contains("/Users/me/Code/repo-sandbox"));
    assert!(!roots.contains("/Users/me/Code"));
    assert!(!roots.contains("repo"));
}

/// Against this machine's real data (read-only):
/// `ACTIVITY_REPO=<path> cargo test -- --ignored`.
#[test]
#[ignore]
fn real_repo_activity() {
    let agents = block_on(list_agents(&AgentsRaw::new())).expect("claude agents");
    let projects = crate::paths::claude_config_dir().unwrap().join("projects");
    let repo = std::env::var("ACTIVITY_REPO").expect("ACTIVITY_REPO=<path to a repo>");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let act = assemble(
        &[PathBuf::from(&repo)],
        &agents,
        &projects,
        &AppRuns::default(),
        now,
    );
    for s in &act.sessions {
        eprintln!(
            "session {} {} {:?} alive={} {:?} {:?}",
            s.session_id, s.kind, s.name, s.alive, s.status, s.last_tool
        );
    }
    for s in &act.subagents {
        eprintln!(
            "subagent {} {:?} {:?} active={} finished={} session={} cwd={:?} tool={:?}",
            s.agent_id,
            s.agent_type,
            s.description,
            s.active,
            s.finished,
            s.session_id,
            s.cwd,
            s.last_tool
        );
    }
}
