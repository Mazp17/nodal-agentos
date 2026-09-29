use super::*;
use crate::testutil::{git_available, init_repo, TempDir};

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
    let root = git_root_of(&sub).unwrap();
    assert_eq!(root.as_deref(), Some(repo.as_path()));
    let outside = t.0.join("not-git");
    std::fs::create_dir_all(&outside).unwrap();
    assert_eq!(git_root_of(outside.to_str().unwrap()).unwrap(), None);
    assert!(require_git_root(outside.to_str().unwrap())
        .unwrap_err()
        .contains("not inside a git repository"));
    assert!(git_root_of("relative").is_err());
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

    let found = git_repos_under(root.to_str().unwrap()).unwrap();
    let expected: Vec<PathBuf> = ["a/b/c", "api", "web/app", "worktrees/api-wt"]
        .iter()
        .map(|r| root.join(r))
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
