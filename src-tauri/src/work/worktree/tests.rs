use super::*;
use crate::util::paths::tests::{git_available, init_repo, TempDir};

#[test]
fn create_reuse_and_cleanup() {
    if !git_available() {
        eprintln!("git not available: skipping");
        return;
    }
    let t = TempDir::new("worktree");
    let repo = t.0.join("repo");
    init_repo(&repo);
    let dir = dir_for(&t.0.join("worktrees"), "repo", "pay-1-logo");
    let wt = ensure(&repo, &dir, "nodal/pay-1-logo", None).unwrap();
    assert_eq!(wt.branch, "nodal/pay-1-logo");
    assert_eq!(wt.base, "main");
    assert!(Path::new(&wt.path).join("README.md").is_file());
    assert_eq!(
        current_base(Path::new(&wt.path)).unwrap(),
        "nodal/pay-1-logo"
    );

    // Reuse: same worktree, without touching what's inside.
    std::fs::write(Path::new(&wt.path).join("wip.txt"), "x").unwrap();
    let again = ensure(&repo, &dir, "nodal/pay-1-logo", Some(&wt)).unwrap();
    assert_eq!(again, wt);
    assert!(Path::new(&wt.path).join("wip.txt").is_file());

    // If it's deleted by hand, it's recreated on the same branch.
    std::fs::remove_dir_all(&wt.path).unwrap();
    let recreated = ensure(&repo, &dir, "nodal/pay-1-logo", Some(&wt)).unwrap();
    assert_eq!(recreated.branch, wt.branch);
    assert!(Path::new(&recreated.path).is_dir());

    // A foreign folder with content isn't overwritten.
    let foreign = t.0.join("worktrees/repo/other");
    std::fs::create_dir_all(&foreign).unwrap();
    std::fs::write(foreign.join("f"), "x").unwrap();
    assert!(ensure(&repo, &foreign, "nodal/other", None)
        .unwrap_err()
        .contains("not a worktree"));

    // Status: foreign folder deleted; the worktree is clean and has no commits of its own.
    std::fs::remove_dir_all(&foreign).unwrap();
    let st = status(&repo, Some(&recreated)).unwrap();
    assert!(st.exists && !st.dirty);
    assert_eq!((st.ahead, st.unpushed), (0, 0));
    assert_eq!(cleanup_blocker(&st), None);
    assert_eq!(status(&repo, None).unwrap(), WorktreeStatus::default());

    let wtp = Path::new(&recreated.path);
    std::fs::write(wtp.join("new.txt"), "x").unwrap();
    let st = status(&repo, Some(&recreated)).unwrap();
    assert!(st.dirty);
    assert!(cleanup_blocker(&st)
        .unwrap()
        .contains("uncommitted changes"));
    git::ok(wtp, &["add", "."]).unwrap();
    git::ok(wtp, &["commit", "-q", "-m", "wip"]).unwrap();
    let st = status(&repo, Some(&recreated)).unwrap();
    assert!(!st.dirty);
    assert_eq!((st.ahead, st.unpushed), (1, 1));
    let msg = cleanup_blocker(&st).unwrap();
    assert!(
        msg.contains("1 unpushed commit:") && msg.contains("force"),
        "{msg}"
    );

    // With the branch pushed to a remote there's nothing left to lose.
    let remote = t.0.join("remote.git");
    git::ok(&t.0, &["init", "-q", "--bare", remote.to_str().unwrap()]).unwrap();
    git::ok(
        &repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    )
    .unwrap();
    git::ok(&repo, &["push", "-q", "origin", "nodal/pay-1-logo"]).unwrap();
    let st = status(&repo, Some(&recreated)).unwrap();
    assert_eq!((st.ahead, st.unpushed), (1, 0));
    assert_eq!(cleanup_blocker(&st), None);

    cleanup(&repo, &recreated).unwrap();
    assert!(!Path::new(&recreated.path).exists());
    assert!(!status(&repo, Some(&recreated)).unwrap().exists);
    assert!(!branch_exists(&repo, "nodal/pay-1-logo").unwrap());
    // Idempotent.
    cleanup(&repo, &recreated).unwrap();
}
