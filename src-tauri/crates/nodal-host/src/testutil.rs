//! Test helpers shared across this crate's test modules and, with the `test-support`
//! feature, by nodal-app's tests.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// A static Tokio runtime, so tests that don't otherwise run inside one (and don't want the
/// overhead of building a fresh one each time) can drive an `async fn`. Replaces
/// `tauri::async_runtime::block_on` for code that moved out of the shell. Built with
/// `new_current_thread` (not `rt-multi-thread`, only a dev-dependency feature): `spawn`ed
/// tasks still run cooperatively, which is all these tests need.
pub fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Builder::new_current_thread().enable_time().enable_io().build().unwrap()).block_on(fut)
}

/// Private temp folder, deleted on drop.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(name: &str) -> Self {
        let d = std::env::temp_dir().join(format!("nodal-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        TempDir(d.canonicalize().unwrap())
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A fake `claude` that appends one line to `count_file` per invocation, so a test can see
/// how many times it actually ran, and prints a valid (empty) `claude agents --json` list.
pub fn fake_claude_counting(dir: &Path, count_file: &Path) -> PathBuf {
    let script = dir.join("claude");
    std::fs::write(&script, format!("#!/bin/sh\necho x >> \"{}\"\necho '[]'\n", count_file.display())).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    script
}

/// Invocations recorded by a [`fake_claude_counting`] script.
pub fn spawn_count(count_file: &Path) -> usize {
    std::fs::read_to_string(count_file).unwrap_or_default().lines().count()
}

pub fn git_available() -> bool {
    std::process::Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// `git init` with an initial commit on `main` and a local test identity.
pub fn init_repo(dir: &Path) {
    let run = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    std::fs::create_dir_all(dir).unwrap();
    run(&["init", "-q", "-b", "main"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "Test"]);
    run(&["config", "commit.gpgsign", "false"]);
    std::fs::write(dir.join("README.md"), "# demo\n").unwrap();
    run(&["add", "."]);
    run(&["commit", "-q", "-m", "init"]);
}
