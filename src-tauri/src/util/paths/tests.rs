use std::path::Path;

use super::*;

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

#[test]
fn dev_data_lives_next_to_the_release_data() {
    let dir = Path::new("/Users/me/Library/Application Support/io.github.mazp17.nodal");
    assert_eq!(
        dev_sibling(dir),
        Path::new("/Users/me/Library/Application Support/io.github.mazp17.nodal.dev")
    );
    // `cargo test` is a debug build unless run with --release.
    assert_eq!(nodal_home().unwrap().ends_with(".nodal-dev"), DEV);
}

#[test]
fn git_root_of_subfolder() {
    if !git_available() {
        eprintln!("git not available: skipping");
        return;
    }
    let t = TempDir::new("git-root");
    let repo = t.0.join("repo");
    init_repo(&repo);
    std::fs::create_dir_all(repo.join("a/b")).unwrap();
    let sub = repo.join("a/b").to_string_lossy().into_owned();
    let root = tauri::async_runtime::block_on(resolve_git_root(sub)).unwrap();
    assert_eq!(root.as_deref(), Some(repo.to_string_lossy().as_ref()));
    let outside = t.0.join("not-git");
    std::fs::create_dir_all(&outside).unwrap();
    assert_eq!(git_root_of(outside.to_str().unwrap()).unwrap(), None);
    assert!(require_git_root(outside.to_str().unwrap())
        .unwrap_err()
        .contains("not inside a git repository"));
    assert!(tauri::async_runtime::block_on(resolve_git_root("relative".into())).is_err());
    assert!(git_root_of("/no/such/dir")
        .unwrap_err()
        .contains("doesn't exist"));
}

#[test]
fn scans_repos_up_to_three_levels_down() {
    if !git_available() {
        eprintln!("git not available: skipping");
        return;
    }
    let t = TempDir::new("scan-repos");
    let root = &t.0;
    for repo in [
        "api",
        "web/app",
        "a/b/c",
        "x/y/z/too-deep",
        "node_modules/pkg",
        ".hidden/repo",
        "api/vendor/lib",
    ] {
        init_repo(&root.join(repo));
    }
    std::fs::create_dir_all(root.join("notes/2026")).unwrap();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(root.join("api"))
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["worktree", "add", "-q", "-b", "wt", "../worktrees/api-wt"]);
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.join("web"), root.join("web-link")).unwrap();

    let found = tauri::async_runtime::block_on(scan_git_repos(root.to_string_lossy().into_owned()))
        .unwrap();
    let expected: Vec<String> = ["a/b/c", "api", "web/app", "worktrees/api-wt"]
        .iter()
        .map(|r| root.join(r).to_string_lossy().into_owned())
        .collect();
    assert_eq!(found, expected);

    assert_eq!(
        git_repos_under(root.join("api").to_str().unwrap()).unwrap(),
        vec![root.join("api")]
    );
    assert_eq!(
        git_repos_under(root.join("notes").to_str().unwrap()).unwrap(),
        Vec::<PathBuf>::new()
    );
    assert!(git_repos_under("/no/such/dir")
        .unwrap_err()
        .contains("doesn't exist"));
    assert!(git_repos_under("relative")
        .unwrap_err()
        .contains("absolute"));
}

#[test]
fn canonical_dir_resolves_symlinks() {
    let t = TempDir::new("canonical-dir");
    std::fs::create_dir_all(t.0.join("real")).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(t.0.join("real"), t.0.join("link")).unwrap();
        assert_eq!(
            canonical_dir(t.0.join("link").to_str().unwrap()).unwrap(),
            t.0.join("real")
        );
    }
    assert_eq!(
        canonical_dir(&format!("  {}  ", t.0.join("real").display())).unwrap(),
        t.0.join("real")
    );
    std::fs::write(t.0.join("file"), "x").unwrap();
    assert!(canonical_dir(t.0.join("file").to_str().unwrap())
        .unwrap_err()
        .contains("doesn't exist"));
}

#[test]
fn expands_home() {
    let h = home().unwrap();
    assert_eq!(expand_home("~/x"), h.join("x"));
    assert_eq!(expand_home("~"), h);
    assert_eq!(expand_home("/a/~"), PathBuf::from("/a/~"));
}
