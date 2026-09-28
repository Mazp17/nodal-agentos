use super::*;
use crate::util::paths::tests::TempDir;

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
    assert!(plan_file(&repo, "../outside/evil.md").unwrap_err().contains("inside"));
    assert!(plan_file(&repo, outside.join("evil.md").to_str().unwrap()).unwrap_err().contains("inside"));
    assert!(plan_file(&repo, "docs/missing.md").unwrap_err().contains("doesn't exist"));
    assert!(plan_file(&repo, "docs/notes.txt").unwrap_err().contains(".md"));
    assert!(plan_file(&repo, "").is_err());
    std::fs::write(repo.join(".git/x.md"), "#").unwrap();
    assert!(plan_file(&repo, ".git/x.md").unwrap_err().contains(".git"));
}

#[cfg(unix)]
#[test]
fn plan_file_symlink_escaping_repo_is_rejected() {
    let (_g, repo, outside) = setup("symlink");
    std::os::unix::fs::symlink(outside.join("evil.md"), repo.join("docs/link.md")).unwrap();
    assert!(plan_file(&repo, "docs/link.md").unwrap_err().contains("inside"));
    std::fs::create_dir_all(repo.join("dir.md")).unwrap();
    assert!(plan_file(&repo, "dir.md").unwrap_err().contains("not a file"));
}

#[test]
fn simple_fields() {
    assert_eq!(title("  Fix   the\n bug ").unwrap(), "Fix the bug");
    assert!(title(" \n ").is_err());
    assert!(title(&"x".repeat(MAX_TITLE_CHARS + 1)).is_err());
    assert!(plan_text("  ").is_err());
    assert!(plan_text("# ok").is_ok());
    assert_eq!(project_key(" pay ").unwrap(), "PAY");
    assert_eq!(project_key("web2").unwrap(), "WEB2");
    for bad in ["P", "PAYMENTS", "2PAY", "PA-Y", ""] {
        assert!(project_key(bad).is_err(), "{bad}");
    }
    assert!(color("#d98c3f").is_ok() && color("#abc").is_ok() && color(PALETTE[0]).is_ok());
    assert!(color("red").is_err() && color("oklch(1;x)").is_err() && color("#12345").is_err());
    assert_eq!(labels(&[" ui ".into(), "UI".into(), "".into(), "api".into()]).unwrap(), ["ui", "api"]);
    assert!(labels(&["x".repeat(41)]).is_err());
    assert_eq!(acceptance(&["a\nb".into(), "  ".into()]).unwrap(), ["a b"]);
    assert!(executor(&Executor::Agent { name: "a b".into(), source: crate::domain::AgentSource::User }).is_err());
    assert!(executor(&Executor::Workflow { name: "-x".into() }).is_err());
    assert!(executor(&Executor::Claude).is_ok());
    assert_eq!(editor(" cursor ").unwrap(), "cursor");
    assert!(editor("rm").is_err());
    assert!(concurrency(0).is_err() && concurrency(17).is_err() && concurrency(3).is_ok());
    assert_eq!(extra_instructions(Some("  ")).unwrap(), None);
    assert_eq!(reviewer(" code-reviewer ").unwrap(), "code-reviewer");
}
