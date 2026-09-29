//! `System::import_legacy` facade tests: `data_dir`/`now`/the connection come from `Core`
//! (`env`, `clock`, `db`); the copy-then-import logic itself is `nodal_store::legacy`,
//! already covered by that crate's own tests.
#![allow(clippy::disallowed_methods)] // writes a fixture legacy config.json to a temp dir

use std::sync::Arc;

use nodal_domain::error::HostError;
use nodal_domain::model::chat::{ChatLive, ChatSpec};
use nodal_domain::ports::ChatRuntime;
use nodal_host::adapters::{HostClaudeCli, HostSessionFiles, SystemClock};
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
    ) -> Result<(), HostError> {
        unreachable!("system doesn't use chats")
    }
    fn respond(
        &self,
        _chat_id: &str,
        _request_id: &str,
        _allow: bool,
        _message: Option<&str>,
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
        claude: Arc::new(HostClaudeCli::new(rt())),
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
