//! Test helpers for nodal-app's own tests: a shared runtime (`rt`, `block_on`), a quick
//! `Env` and a no-op `ChangeNotifier`. Not all of this is used yet: contexts are still
//! skeletons, waves 3a-3c bring the tests that need it.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use tokio::runtime::{Handle, Runtime};

use nodal_domain::model::events::ChangeKind;
use nodal_domain::ports::ChangeNotifier;
use nodal_host::adapters::{HostClaudeConfig, HostGit, HostLocalFs, HostPlanFiles};

use crate::core::Env;

fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .enable_io()
            .build()
            .expect("tokio runtime")
    })
}

/// A `Handle` into the shared test runtime, for ports that need one (e.g. `HostClaudeCli::new`).
pub fn rt() -> Handle {
    runtime().handle().clone()
}

/// Drives an `async fn` from a synchronous test. Replaces `tauri::async_runtime::block_on`.
pub fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    runtime().block_on(fut)
}

/// `Env` rooted at `root`, with the real (blocking, synchronous) host adapters.
pub fn env_for(root: &std::path::Path, claude_dir: Option<PathBuf>) -> Env {
    Env {
        data_dir: root.join("data"),
        worktrees_root: root.join("wt"),
        claude_dir,
        plans: Arc::new(HostPlanFiles),
        fs: Arc::new(HostLocalFs),
        git: Arc::new(HostGit),
        claude_config: Arc::new(HostClaudeConfig),
    }
}

/// A `ChangeNotifier` that does nothing (tests that don't check notifications).
#[derive(Default)]
pub struct NoopNotifier;

impl ChangeNotifier for NoopNotifier {
    fn notify(&self, _kind: ChangeKind, _project_id: Option<&str>) {}
}
