use super::*;
use crate::testutil::TempDir;

/// Two sibling folders: `repo` and `outside`.
fn setup(name: &str) -> (TempDir, PathBuf, PathBuf) {
    let t = TempDir::new(name);
    let repo = t.0.join("repo");
    let outside = t.0.join("outside");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::create_dir_all(repo.join("docs")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(repo.join("docs/plan.md"), "# Plan").unwrap();
    std::fs::write(repo.join("docs/notes.txt"), "x").unwrap();
    std::fs::write(outside.join("evil.md"), "# Evil").unwrap();
    (t, repo, outside)
}

#[test]
fn plan_file_rules() {
    let (_g, repo, outside) = setup("plan");
    let ok = repo.join("docs/plan.md");
    assert_eq!(plan_file(&repo, ok.to_str().unwrap()).unwrap(), ok);
    assert_eq!(plan_file(&repo, "docs/plan.md").unwrap(), ok);
    assert_eq!(plan_file(&repo, "./docs/../docs/plan.md").unwrap(), ok);
    assert!(plan_file(&repo, "../outside/evil.md")
        .unwrap_err()
        .contains("inside"));
    assert!(plan_file(&repo, outside.join("evil.md").to_str().unwrap())
        .unwrap_err()
        .contains("inside"));
    assert!(plan_file(&repo, "docs/missing.md")
        .unwrap_err()
        .contains("doesn't exist"));
    assert!(plan_file(&repo, "docs/notes.txt")
        .unwrap_err()
        .contains(".md"));
    assert!(plan_file(&repo, "").is_err());
    std::fs::write(repo.join(".git/x.md"), "#").unwrap();
    assert!(plan_file(&repo, ".git/x.md").unwrap_err().contains(".git"));
}

#[cfg(unix)]
#[test]
fn plan_file_symlink_escaping_repo_is_rejected() {
    let (_g, repo, outside) = setup("symlink");
    std::os::unix::fs::symlink(outside.join("evil.md"), repo.join("docs/link.md")).unwrap();
    assert!(plan_file(&repo, "docs/link.md")
        .unwrap_err()
        .contains("inside"));
    std::fs::create_dir_all(repo.join("dir.md")).unwrap();
    assert!(plan_file(&repo, "dir.md")
        .unwrap_err()
        .contains("not a file"));
}
