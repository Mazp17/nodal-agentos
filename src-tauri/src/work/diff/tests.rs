use super::*;
use crate::util::paths::tests::{git_available, init_repo, TempDir};

#[test]
fn diff_of_a_worktree_branch_includes_uncommitted_and_untracked() {
    if !git_available() {
        eprintln!("git not available: skipping");
        return;
    }
    let t = TempDir::new("diff");
    let repo = t.0.join("repo");
    init_repo(&repo);
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["checkout", "-q", "-b", "feature"]);
    std::fs::write(repo.join("README.md"), "# demo\ncommitted\n").unwrap();
    git(&["commit", "-qam", "change"]);
    let (patch, dirty) = collect(&repo, Some("main")).unwrap();
    assert!(!dirty);
    assert_eq!(current_branch(&repo).as_deref(), Some("feature"));
    let log = commits(&repo, "main", "HEAD").unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(
        (log[0].subject.as_str(), log[0].author.as_str()),
        ("change", "Test")
    );
    assert!(log[0].sha.starts_with(&log[0].short_sha) && log[0].at > 0);
    assert_eq!(commits(&repo, "main", "feature").unwrap(), log);
    assert!(commits(&repo, "feature", "HEAD").unwrap().is_empty());
    assert!(commits(&repo, "--all", "HEAD").is_err());
    let files = parse(&patch);
    assert_eq!(files.len(), 1);
    assert_eq!((files[0].additions, files[0].deletions), (1, 0));
    // The same branch seen from outside (like a workflow's).
    assert_eq!(
        parse(&collect_branch(&repo, "main", "feature").unwrap()).len(),
        1
    );
    assert!(collect_branch(&repo, "main", "--output=x").is_err());

    std::fs::write(repo.join("README.md"), "# demo\ncommitted\nwip\n").unwrap();
    std::fs::write(repo.join("new file.txt"), "hello\n").unwrap();
    let (patch, dirty) = collect(&repo, Some("main")).unwrap();
    assert!(dirty);
    let files = parse(&patch);
    let paths: Vec<(&str, DiffFileStatus, u32)> = files
        .iter()
        .map(|f| (f.path.as_str(), f.status, f.additions))
        .collect();
    assert_eq!(
        paths,
        [
            ("README.md", DiffFileStatus::Modified, 2),
            ("new file.txt", DiffFileStatus::Added, 1)
        ]
    );
    // No base (in_place): only uncommitted work.
    let (patch, _) = collect(&repo, None).unwrap();
    let files = parse(&patch);
    assert_eq!(files[0].additions, 1);
    assert!(collect(&repo, Some("no-such-branch")).is_err());
}
