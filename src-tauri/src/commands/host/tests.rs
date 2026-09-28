#[test]
fn git_version_reports_git() {
    if !crate::util::paths::tests::git_available() {
        return;
    }
    let v = tauri::async_runtime::block_on(super::git_version()).unwrap();
    assert!(v.starts_with("git version "), "{v}");
}

/// Wrapper-level coverage for `resolve_git_root`: the command parses/serializes as a plain
/// `String` in and `Option<String>` out, on top of `nodal_host::paths::git_root_of` (whose own
/// unit tests live in that crate now).
#[test]
fn resolve_git_root_finds_the_repo_root_or_none() {
    use crate::util::paths::tests::{git_available, init_repo, TempDir};

    if !git_available() {
        return;
    }
    let t = TempDir::new("resolve-git-root-cmd");
    let repo = t.0.join("repo");
    init_repo(&repo);
    std::fs::create_dir_all(repo.join("a/b")).unwrap();
    let sub = repo.join("a/b").to_string_lossy().into_owned();

    let root = tauri::async_runtime::block_on(super::resolve_git_root(sub)).unwrap();
    assert_eq!(root.as_deref(), Some(repo.to_string_lossy().as_ref()));

    let outside = t.0.join("not-git");
    std::fs::create_dir_all(&outside).unwrap();
    let root = tauri::async_runtime::block_on(super::resolve_git_root(outside.to_string_lossy().into_owned())).unwrap();
    assert_eq!(root, None);
}

/// Wrapper-level coverage for `scan_git_repos`, on top of `nodal_host::paths::git_repos_under`.
#[test]
fn scan_git_repos_finds_repos_under_the_root() {
    use crate::util::paths::tests::{git_available, init_repo, TempDir};

    if !git_available() {
        return;
    }
    let t = TempDir::new("scan-git-repos-cmd");
    init_repo(&t.0.join("api"));
    init_repo(&t.0.join("web/app"));

    let found = tauri::async_runtime::block_on(super::scan_git_repos(t.0.to_string_lossy().into_owned())).unwrap();
    let expected: Vec<String> = ["api", "web/app"].iter().map(|r| t.0.join(r).to_string_lossy().into_owned()).collect();
    assert_eq!(found, expected);
}
