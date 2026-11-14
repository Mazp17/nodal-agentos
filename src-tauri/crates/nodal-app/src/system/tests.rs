//! `System::import_legacy` facade tests: `data_dir`/`now`/the connection come from `Core`
//! (`env`, `clock`, `db`); the copy-then-import logic itself is `nodal_store::legacy`,
//! already covered by that crate's own tests.
#![allow(clippy::disallowed_methods)] // writes a fixture legacy config.json to a temp dir

use std::sync::Arc;

use nodal_domain::error::HostError;
use nodal_domain::model::chat::{ChatLive, ChatSpec};
use nodal_domain::ports::ChatRuntime;
use nodal_host::adapters::{HostSessionFiles, SystemClock};
use nodal_host::testutil::TempDir;
use nodal_store::Db;

use super::System;
use crate::core::Core;
use crate::testutil::{block_on, env_for, rt, NoopNotifier};

/// `System` never calls into chats; a fake that panics if that ever changes.
struct UnusedChats;

impl ChatRuntime for UnusedChats {
    fn send(
        &self,
        _chat_id: &str,
        _project_id: &str,
        _session_id: Option<&str>,
        _spec: ChatSpec,
        _text: &str,
        _images: &[nodal_domain::model::chat::ImageInput],
    ) -> Result<(), HostError> {
        unreachable!("system doesn't use chats")
    }
    fn respond(
        &self,
        _chat_id: &str,
        _request_id: &str,
        _allow: bool,
        _message: Option<&str>,
        _answers: Option<&std::collections::BTreeMap<String, String>>,
    ) -> Result<(), HostError> {
        unreachable!("system doesn't use chats")
    }
    fn interrupt(&self, _chat_id: &str) -> Result<(), HostError> {
        unreachable!("system doesn't use chats")
    }
    fn stop(&self, _chat_id: &str) {}
    fn stop_project(&self, _project_id: &str) {}
    fn live(&self, _chat_id: &str) -> ChatLive {
        unreachable!("system doesn't use chats")
    }
    fn start_reaper(&self) {}
}

fn system_with(root: &std::path::Path, db: Db) -> System {
    let core = Arc::new(Core {
        db,
        env: env_for(root, None),
        rt: rt(),
        clock: Arc::new(SystemClock),
        sessions: Arc::new(HostSessionFiles),
        notifier: Arc::new(NoopNotifier),
        chats: Arc::new(UnusedChats),
    });
    System::new(core)
}

#[test]
fn import_legacy_rejects_a_folder_with_no_known_legacy_files() {
    let root = TempDir::new("system-import-empty");
    let src = TempDir::new("system-import-empty-src");
    let db = Db::open_in_memory().expect("in-memory db");
    let system = system_with(&root.0, db);

    let err = block_on(system.import_legacy(src.0.display().to_string())).unwrap_err();
    assert!(
        err.to_string()
            .contains("doesn't look like a data folder from the previous version"),
        "{err}"
    );
}

#[test]
fn import_legacy_uses_core_env_data_dir_now_and_db() {
    let root = TempDir::new("system-import-ok");
    let src = TempDir::new("system-import-ok-src");
    std::fs::write(
        src.0.join("config.json"),
        r#"{"repos":[{"path":"/tmp/nodal-legacy-test-repo"}]}"#,
    )
    .unwrap();

    let db = Db::open_in_memory().expect("in-memory db");
    let system = system_with(&root.0, db.clone());

    // Leading/trailing whitespace, like the old `folder.trim()`.
    let report = block_on(system.import_legacy(format!("  {}  ", src.0.display())))
        .expect("import succeeds");
    assert_eq!(report.projects, 1);
    assert_eq!(report.repos, 1);
    assert_eq!(report.already_imported, 0);
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);

    // `data_dir` came from `core.env`, not from some other path.
    let data_dir = root.0.join("data");
    assert!(
        report
            .backup_dir
            .starts_with(&data_dir.display().to_string()),
        "{} not under {}",
        report.backup_dir,
        data_dir.display()
    );

    // The row landed in `core.db`, not in some other connection.
    let repos: i64 = db
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM repos WHERE path = ?1",
            ["/tmp/nodal-legacy-test-repo"],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(repos, 1);
}

#[test]
fn repo_file_commands_only_accept_the_repos_own_worktrees() {
    use std::path::Path;

    use nodal_domain::testutil::{project_of, repo_of};
    use nodal_host::testutil::{git_available, init_repo};
    use nodal_store::rows::{insert_project, insert_repo};

    if !git_available() {
        return;
    }
    let root = TempDir::new("system-worktrees");
    let repo = root.0.join("repo");
    init_repo(&repo);
    let wt = root.0.join("wt");
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["worktree", "add", "-q", "-b", "side", wt.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(status.success());
    std::fs::write(wt.join("only-in-wt.txt"), "x").unwrap();

    let db = Db::open_in_memory().unwrap();
    {
        let c = db.lock().unwrap();
        insert_project(&c, &project_of("p1", "PAY")).unwrap();
        insert_repo(&c, &repo_of("r1", "p1", repo.to_str().unwrap())).unwrap();
    }
    let system = system_with(&root.0, db);
    let list = |wt: Option<&Path>| {
        block_on(system.list_repo_dir("r1".into(), "".into(), wt.map(|p| p.display().to_string())))
    };

    let main = list(None).unwrap();
    assert!(main.iter().all(|e| e.name != "only-in-wt.txt"));
    assert!(list(Some(&wt)).unwrap().iter().any(|e| e.name == "only-in-wt.txt"));
    // Anything else, even an existing folder, is not a worktree of this repo.
    for bad in [root.0.as_path(), Path::new("/"), Path::new("/nope")] {
        let err = list(Some(bad)).unwrap_err().to_string();
        assert_eq!(err, "Not a worktree of this repo.");
    }
    let err = block_on(system.revert_repo_file("r1".into(), "x".into(), Some("/".into()))).unwrap_err();
    assert_eq!(err.to_string(), "Not a worktree of this repo.");
}

/// A system with one repo `r1` at `<root>/repo` (git-initialised), or `None` without git.
fn system_with_repo(name: &str) -> Option<(TempDir, std::path::PathBuf, System)> {
    use nodal_domain::testutil::{project_of, repo_of};
    use nodal_host::testutil::{git_available, init_repo};
    use nodal_store::rows::{insert_project, insert_repo};

    if !git_available() {
        return None;
    }
    let root = TempDir::new(name);
    let repo = root.0.join("repo");
    init_repo(&repo);
    let db = Db::open_in_memory().unwrap();
    {
        let c = db.lock().unwrap();
        insert_project(&c, &project_of("p1", "PAY")).unwrap();
        insert_repo(&c, &repo_of("r1", "p1", repo.to_str().unwrap())).unwrap();
    }
    let system = system_with(&root.0, db);
    Some((root, repo, system))
}

fn git_in(dir: &std::path::Path, args: &[&str]) {
    let st = std::process::Command::new("git").arg("-C").arg(dir).args(args).status().unwrap();
    assert!(st.success(), "git {args:?}");
}

#[test]
fn repo_commit_stages_tracked_and_untracked_files() {
    let Some((_root, repo, system)) = system_with_repo("system-commit") else { return };
    std::fs::write(repo.join("README.md"), "# changed\n").unwrap();
    std::fs::write(repo.join("new.txt"), "new\n").unwrap();

    let info = block_on(system.repo_commit("r1".into(), None, "  add things \n".into(), vec![])).unwrap();
    assert_eq!(info.subject, "add things");
    let tree = std::process::Command::new("git").arg("-C").arg(&repo).args(["status", "--porcelain"]).output().unwrap();
    assert!(tree.stdout.is_empty(), "tree not clean");
    let shown = std::process::Command::new("git").arg("-C").arg(&repo).args(["show", "--name-only", "--format="]).output().unwrap();
    let files = String::from_utf8_lossy(&shown.stdout).into_owned();
    assert!(files.contains("README.md") && files.contains("new.txt"), "{files}");
}

#[test]
fn repo_commit_with_paths_commits_only_those() {
    let Some((_root, repo, system)) = system_with_repo("system-commit-paths") else { return };
    std::fs::write(repo.join("a.txt"), "a\n").unwrap();
    std::fs::write(repo.join("b.txt"), "b\n").unwrap();
    block_on(system.repo_commit("r1".into(), None, "only a".into(), vec!["a.txt".into()])).unwrap();
    let shown = std::process::Command::new("git").arg("-C").arg(&repo).args(["show", "--name-only", "--format="]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&shown.stdout).trim(), "a.txt");
    let left = std::process::Command::new("git").arg("-C").arg(&repo).args(["status", "--porcelain"]).output().unwrap();
    assert!(String::from_utf8_lossy(&left.stdout).contains("b.txt"), "b.txt should still be pending");
    assert!(block_on(system.repo_commit("r1".into(), None, "x".into(), vec!["../etc".into()])).is_err());
}

#[test]
fn repo_commit_rejects_an_empty_message_and_a_clean_tree() {
    let Some((_root, repo, system)) = system_with_repo("system-commit-bad") else { return };
    std::fs::write(repo.join("new.txt"), "new\n").unwrap();
    let err = block_on(system.repo_commit("r1".into(), None, "  \n".into(), vec![])).unwrap_err();
    assert_eq!(err.to_string(), "Write a commit message.");
    block_on(system.repo_commit("r1".into(), None, "first".into(), vec![])).unwrap();
    let err = block_on(system.repo_commit("r1".into(), None, "second".into(), vec![])).unwrap_err();
    assert_eq!(err.to_string(), "Nothing to commit.");
}

#[test]
fn repo_open_pr_refuses_before_pushing() {
    let Some((root, repo, system)) = system_with_repo("system-open-pr") else { return };
    let pr = |wt: Option<&std::path::Path>, title: &str| {
        block_on(system.repo_open_pr("r1".into(), wt.map(|p| p.display().to_string()), title.into(), "".into()))
            .unwrap_err()
            .to_string()
    };
    assert_eq!(pr(None, " "), "Write a title for the pull request.");
    // The repo's own checkout with no known remote default branch.
    assert_eq!(pr(None, "t"), "Open a PR from a worktree branch.");
    // The remote default branch is the checked-out branch.
    git_in(&repo, &["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"]);
    assert!(pr(None, "t").contains("is the base branch"));
    // A detached worktree.
    let wt = root.0.join("wt-detached");
    git_in(&repo, &["worktree", "add", "-q", "--detach", wt.to_str().unwrap()]);
    assert!(pr(Some(&wt), "t").contains("HEAD is detached"));
}
