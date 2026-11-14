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
        block_on(system.repo_open_pr("r1".into(), wt.map(|p| p.display().to_string()), title.into(), "".into(), None))
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

#[cfg(unix)]
mod pr_panel {
    use nodal_domain::model::{Chat, LaunchOptions};
    use nodal_domain::model::{BranchPr, ChatPr, PrState};
    use nodal_host::testutil::{fake_gh_calls, fake_gh_fail, fake_gh_reply, install_fake_gh};
    use nodal_store::board::{branch_prs, chat_prs};
    use nodal_store::{rows, with_db};

    use super::*;

    const URL: &str = "https://github.com/o/r/pull/7";

    fn pr_json(state: &str) -> String {
        format!(r#"{{"headRefOid":"","isDraft":false,"number":7,"state":"{state}","url":"{URL}"}}"#)
    }

    /// A system on repo `r1` with branch `feat` checked out, chat `c1` and the fake `gh`.
    fn setup(name: &str) -> Option<(TempDir, std::path::PathBuf, System)> {
        let (root, repo, system) = system_with_repo(name)?;
        install_fake_gh();
        git_in(&repo, &["checkout", "-q", "-b", "feat"]);
        let chat = Chat {
            id: "c1".into(),
            project_id: "p1".into(),
            repo_id: Some("r1".into()),
            title: None,
            session_title: None,
            agent: None,
            task_id: None,
            worktree: None,
            archived_at: None,
            session_id: None,
            launch: LaunchOptions::default(),
            created_at: 1,
            updated_at: 1,
            pinned_at: None,
        };
        rows::insert_chat(&system.core.db.lock().unwrap(), &chat).unwrap();
        Some((root, repo, system))
    }

    fn stored(system: &System) -> Option<(BranchPr, i64)> {
        block_on(with_db(&system.core.db, |c| branch_prs::get(c, "r1", "feat"))).unwrap()
    }

    #[test]
    fn repo_pr_looks_up_once_then_reuses_the_fresh_row() {
        let Some((_root, repo, system)) = setup("system-repo-pr") else { return };
        let pr = || block_on(system.repo_pr("r1".into(), None, None)).unwrap();
        fake_gh_reply(&repo, "pr_list", &format!("[{}]", pr_json("OPEN")));
        let found = pr().unwrap();
        assert_eq!((found.number, found.state, found.manual, found.branch.as_str()), (7, PrState::Open, false, "feat"));
        assert_eq!(stored(&system).unwrap().0, found);
        assert_eq!(fake_gh_calls(&repo), 1);
        // Fresh: gh isn't asked again, even though it would now say something else.
        fake_gh_reply(&repo, "pr_list", &format!("[{}]", pr_json("MERGED")));
        assert_eq!(pr().unwrap().state, PrState::Open);
        assert_eq!(fake_gh_calls(&repo), 1);
        // `force` asks again and keeps the new answer.
        let forced = block_on(system.repo_pr("r1".into(), None, Some(true))).unwrap().unwrap();
        assert_eq!(forced.state, PrState::Merged);
        assert_eq!(stored(&system).unwrap().0.state, PrState::Merged);
    }

    #[test]
    fn repo_pr_drops_a_stale_auto_row_and_survives_gh_failing() {
        let Some((_root, repo, system)) = setup("system-repo-pr-stale") else { return };
        let old = BranchPr { url: URL.into(), number: 7, state: PrState::Open, head: None, manual: false, branch: "feat".into() };
        let p = old.clone();
        block_on(with_db(&system.core.db, move |c| branch_prs::upsert(c, "r1", &p, 1))).unwrap();
        // gh is down: the stored (stale) PR comes back.
        fake_gh_fail(&repo, "pr_list", "not logged in");
        assert_eq!(block_on(system.repo_pr("r1".into(), None, None)).unwrap(), Some(old));
        // gh says there is no PR any more: the auto row goes.
        fake_gh_reply(&repo, "pr_list", "[]");
        assert_eq!(block_on(system.repo_pr("r1".into(), None, None)).unwrap(), None);
        assert!(stored(&system).is_none());
    }

    #[test]
    fn repo_pr_has_nothing_to_show_on_the_default_branch() {
        let Some((_root, repo, system)) = setup("system-repo-pr-main") else { return };
        git_in(&repo, &["checkout", "-q", "main"]);
        git_in(&repo, &["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"]);
        assert_eq!(block_on(system.repo_pr("r1".into(), None, Some(true))).unwrap(), None);
        assert_eq!(fake_gh_calls(&repo), 0);
        let err = block_on(system.repo_link_pr("r1".into(), None, "7".into(), None)).unwrap_err();
        assert_eq!(err.to_string(), "Check out a branch to link a PR.");
        block_on(system.repo_unlink_pr("r1".into(), None)).unwrap();
    }

    #[test]
    fn repo_link_pr_stores_a_manual_row_and_records_it_for_the_chat() {
        let Some((_root, repo, system)) = setup("system-link-pr") else { return };
        fake_gh_reply(&repo, "pr_view", &pr_json("OPEN"));
        let err = block_on(system.repo_link_pr("r1".into(), None, "--web".into(), None)).unwrap_err();
        assert!(err.to_string().contains("Enter a PR number"), "{err}");
        let linked = block_on(system.repo_link_pr("r1".into(), None, " 7 ".into(), Some("c1".into()))).unwrap();
        assert!(linked.manual && linked.number == 7 && linked.branch == "feat");
        assert_eq!(stored(&system).unwrap().0, linked);
        let listed = block_on(system.chat_prs("c1".into())).unwrap();
        assert_eq!((listed.len(), listed[0].url.as_str(), listed[0].branch.as_deref()), (1, URL, Some("feat")));
        // Linking again doesn't duplicate the chat's PR.
        block_on(system.repo_link_pr("r1".into(), None, "7".into(), Some("c1".into()))).unwrap();
        assert_eq!(block_on(system.chat_prs("c1".into())).unwrap().len(), 1);
        // A chat that doesn't exist is logged, not fatal.
        block_on(system.repo_link_pr("r1".into(), None, "7".into(), Some("nope".into()))).unwrap();
        assert!(block_on(system.chat_prs("../x".into())).is_err());

        // A manual row is never replaced by a branch lookup; forcing re-reads that very PR.
        let calls = fake_gh_calls(&repo);
        assert_eq!(block_on(system.repo_pr("r1".into(), None, None)).unwrap(), Some(linked.clone()));
        assert_eq!(fake_gh_calls(&repo), calls);
        fake_gh_reply(&repo, "pr_view", &pr_json("CLOSED"));
        let forced = block_on(system.repo_pr("r1".into(), None, Some(true))).unwrap().unwrap();
        assert!(forced.manual && forced.state == PrState::Closed);

        block_on(system.repo_unlink_pr("r1".into(), None)).unwrap();
        assert!(stored(&system).is_none());
    }

    #[test]
    fn repo_merge_details_and_check_log_go_through_gh() {
        let Some((_root, repo, system)) = setup("system-merge-pr") else { return };
        fake_gh_reply(&repo, "pr_list", &format!("[{}]", pr_json("MERGED")));
        fake_gh_reply(&repo, "pr_merge", "");
        fake_gh_reply(&repo, "run_view", "boom\n");
        block_on(system.repo_merge_pr("r1".into(), None, 7, "squash".into())).unwrap();
        assert_eq!(stored(&system).unwrap().0.state, PrState::Merged, "the PR is re-read after merging");
        let err = block_on(system.repo_merge_pr("r1".into(), None, 7, "yolo".into())).unwrap_err();
        assert!(err.to_string().contains("Unknown merge method"), "{err}");
        assert_eq!(block_on(system.repo_check_log("r1".into(), None, "12".into())).unwrap(), "boom");
        assert!(block_on(system.repo_check_log("r1".into(), None, "x".into())).is_err());
        assert!(block_on(system.repo_pr_details("r1".into(), None, 7)).is_err(), "gh has no reply for it yet");
        fake_gh_reply(&repo, "pr_view", &pr_json("OPEN"));
        assert_eq!(block_on(system.repo_pr_details("r1".into(), None, 7)).unwrap()["number"], 7);
    }

    #[test]
    fn chat_prs_lists_what_was_recorded() {
        let Some((_root, _repo, system)) = setup("system-chat-prs") else { return };
        assert!(block_on(system.chat_prs("c1".into())).unwrap().is_empty());
        let pr = ChatPr { repo_id: "r1".into(), url: URL.into(), number: 7, branch: None, created_at: 5 };
        let p = pr.clone();
        assert!(block_on(with_db(&system.core.db, move |c| chat_prs::insert_if_new(c, "c1", &p))).unwrap());
        assert_eq!(block_on(system.chat_prs("c1".into())).unwrap(), [pr]);
    }
}
