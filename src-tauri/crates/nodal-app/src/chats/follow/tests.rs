#![allow(clippy::disallowed_methods)] // real git repos on disk

use serde_json::json;

use nodal_domain::model::Chat;
use nodal_domain::ports::Git;
use nodal_domain::testutil::{project_of, repo_of, task_of};
use nodal_host::adapters::HostGit;
use nodal_store::rows::{insert_repo, insert_task};
use nodal_store::{chats, rows, Db};

use super::*;

fn call(name: &str, input: Value) -> (String, Value) {
    (name.to_string(), input)
}

fn cands(paths: &[&str]) -> Vec<(String, String)> {
    paths
        .iter()
        .map(|p| ("r1".to_string(), p.to_string()))
        .collect()
}

#[test]
fn pick_takes_writes_and_creation_but_not_mentions() {
    let c = cands(&["/w/feat-long-name", "/w/other"]);
    let wt = |calls: &[(String, Value)]| pick(calls, &c, |_| false).map(|(_, p)| p.clone());

    assert_eq!(
        wt(&[call(
            "Edit",
            json!({"file_path": "/w/feat-long-name/README.md"})
        )])
        .as_deref(),
        Some("/w/feat-long-name")
    );
    assert_eq!(
        wt(&[call(
            "Bash",
            json!({"command": "N=feat-long-name && git worktree add -b $N ../$N && cd ../$N"})
        )])
        .as_deref(),
        Some("/w/feat-long-name")
    );
    assert_eq!(
        wt(&[call(
            "Bash",
            json!({"command": "cd /w/other && echo x >> a"})
        )])
        .as_deref(),
        Some("/w/other")
    );
    assert_eq!(
        wt(&[call("EnterWorktree", json!({"name": "other"}))]).as_deref(),
        Some("/w/other")
    );
    // The last one touched wins.
    assert_eq!(
        wt(&[
            call("Write", json!({"file_path": "/w/other/a"})),
            call("Edit", json!({"file_path": "/w/feat-long-name/b"}))
        ])
        .as_deref(),
        Some("/w/feat-long-name")
    );
    // Listing, reading, or a name inside a longer one: no.
    assert_eq!(
        wt(&[call("Bash", json!({"command": "git worktree list"}))]),
        None
    );
    assert_eq!(
        wt(&[call("Read", json!({"file_path": "/w/other/a"}))]),
        None
    );
    assert_eq!(
        wt(&[call(
            "Bash",
            json!({"command": "git worktree add ../other-2"})
        )]),
        None
    );
    assert_eq!(
        wt(&[call("Edit", json!({"file_path": "/w/other-2/a"}))]),
        None
    );

    // A path built at runtime names nothing: the one worktree added during the turn wins,
    // unless several were.
    let bash = [call("Bash", json!({"command": "D=../x-${B##*/}; git worktree add -b $B $D main"}))];
    let pick_added = |added: &[&str]| pick(&bash, &c, |p| added.contains(&p)).map(|(_, p)| p.clone());
    assert_eq!(pick_added(&["/w/other"]).as_deref(), Some("/w/other"));
    assert_eq!(pick_added(&["/w/other", "/w/feat-long-name"]), None);
    assert_eq!(pick_added(&[]), None);
}

/// Adds a worktree and returns its path as `git worktree list` reports it, which is what
/// `follow_worktree` matches against (macOS temp dirs sit behind a symlink; `canonicalize`
/// would give Windows a `\\?\` path git can't use).
fn added_at(git: &HostGit, repo: &std::path::Path, dir: &std::path::Path, branch: &str) -> String {
    git.ensure_worktree(repo, dir, branch, None).unwrap();
    let listed = git.list_worktrees(repo).unwrap();
    listed.into_iter().find(|w| w.branch.as_deref() == Some(branch)).unwrap().path
}

fn chat(id: &str, repo_id: Option<&str>, task_id: Option<&str>, worktree: Option<String>) -> Chat {
    Chat {
        id: id.into(),
        project_id: "p1".into(),
        repo_id: repo_id.map(String::from),
        title: None,
        session_title: None,
        session_id: None,
        agent: None,
        task_id: task_id.map(String::from),
        worktree,
        archived_at: None,
        launch: Default::default(),
        created_at: 1,
        // After every worktree here: only the touch rule applies (see the "added" test).
        updated_at: i64::MAX,
    }
}

#[test]
fn follow_worktree_moves_a_free_chat_into_the_worktree_it_worked_in() {
    let t = nodal_host::testutil::TempDir::new("follow-worktree");
    let root = t.0.clone();
    let repo_dir = root.join("repo");
    nodal_host::testutil::init_repo(&repo_dir);
    let git = HostGit;
    let mk = |name: &str| added_at(&git, &repo_dir, &root.join(name), name);
    let (mine, tasks_wt, other_chats) = (mk("mine"), mk("tasks-wt"), mk("other-chats"));

    let db = Db::open_in_memory().unwrap();
    {
        let c = db.lock().unwrap();
        rows::insert_project(&c, &project_of("p1", "PAY")).unwrap();
        insert_repo(&c, &repo_of("r1", "p1", &repo_dir.to_string_lossy())).unwrap();
        let mut task = task_of("t1");
        task.worktree = Some(nodal_domain::model::WorktreeRef {
            path: tasks_wt.clone(),
            branch: "tasks-wt".into(),
            base: "main".into(),
        });
        insert_task(&c, &task).unwrap();
        rows::insert_chat(&c, &chat("c1", None, None, None)).unwrap();
        rows::insert_chat(&c, &chat("c2", Some("r1"), None, Some(other_chats.clone()))).unwrap();
        rows::insert_chat(&c, &chat("c3", Some("r1"), Some("t1"), None)).unwrap();
    }
    let get = |id: &str| chats::get(&db.lock().unwrap(), id).unwrap();
    let edit = |p: &str| vec![call("Edit", json!({"file_path": format!("{p}/a.txt")}))];

    // Worktrees owned by a task or another chat, and the main checkout, are never taken.
    assert!(!follow_worktree(&db, &git, &edit(&tasks_wt), "c1").unwrap());
    assert!(!follow_worktree(&db, &git, &edit(&other_chats), "c1").unwrap());
    assert!(!follow_worktree(&db, &git, &edit(&repo_dir.to_string_lossy()), "c1").unwrap());
    assert_eq!(get("c1").worktree, None);

    // An "All repos" chat takes the worktree and its repo.
    assert!(follow_worktree(&db, &git, &edit(&mine), "c1").unwrap());
    assert_eq!(
        (get("c1").worktree, get("c1").repo_id),
        (Some(mine.clone()), Some("r1".into()))
    );
    // Already there: no change.
    assert!(!follow_worktree(&db, &git, &edit(&mine), "c1").unwrap());

    // A task chat keeps its task's worktree.
    assert!(!follow_worktree(&db, &git, &edit(&mine), "c3").unwrap());
}

#[test]
fn follow_worktree_takes_the_one_added_during_the_turn_even_unnamed() {
    let t = nodal_host::testutil::TempDir::new("follow-added");
    let root = t.0.clone();
    let repo_dir = root.join("repo");
    nodal_host::testutil::init_repo(&repo_dir);
    let git = HostGit;
    let db = Db::open_in_memory().unwrap();
    {
        let c = db.lock().unwrap();
        rows::insert_project(&c, &project_of("p1", "PAY")).unwrap();
        insert_repo(&c, &repo_of("r1", "p1", &repo_dir.to_string_lossy())).unwrap();
        let mut chat = chat("c1", Some("r1"), None, None);
        chat.updated_at = 0; // the message was sent before the worktree below was added
        rows::insert_chat(&c, &chat).unwrap();
    }
    let added = added_at(&git, &repo_dir, &root.join("demo-204105"), "demo-204105");
    let bash = [call("Bash", json!({"command": "B=demo-$(date +%H%M%S); git worktree add -b $B ../${B##*/} main"}))];
    assert!(follow_worktree(&db, &git, &bash, "c1").unwrap());
    assert_eq!(chats::get(&db.lock().unwrap(), "c1").unwrap().worktree, Some(added));
}
