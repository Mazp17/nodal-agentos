use super::*;
use crate::testutil::{git_available, init_repo, TempDir};

#[test]
fn numstat_counts_binary_rows_as_files_with_zero_lines() {
    let s = parse_numstat("3\t1\tsrc/a.rs\n-\t-\timg.png\n0\t2\tb.txt\n");
    assert_eq!(s, DiffStat { files: 3, additions: 3, deletions: 3 });
    assert_eq!(parse_numstat(""), DiffStat::default());
}

#[test]
fn changes_cover_tracked_untracked_and_ignored() {
    if !git_available() {
        eprintln!("git not available: skipping");
        return;
    }
    let t = TempDir::new("stat");
    let repo = t.0.join("repo");
    init_repo(&repo);
    assert_eq!(changes(&repo), DiffStat::default());
    assert!(!is_worktree(&repo));

    std::fs::write(repo.join("README.md"), "# demo\nmore\n").unwrap(); // +1
    std::fs::write(repo.join(".gitignore"), "*.log\n").unwrap(); // untracked, 1 line
    std::fs::write(repo.join("x.log"), "ignored\n").unwrap();
    std::fs::write(repo.join("new.txt"), "a\nb\nc").unwrap(); // 3 lines, no final newline
    std::fs::write(repo.join("bin.dat"), [0u8, 1, 2]).unwrap(); // file, 0 lines
    assert_eq!(changes(&repo), DiffStat { files: 4, additions: 1 + 1 + 3, deletions: 0 });

    // A linked worktree has `.git` as a file.
    let wt = t.0.join("wt");
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["worktree", "add", "-q", "-b", "side"])
        .arg(&wt)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(is_worktree(&wt));
    assert_eq!(changes(&wt), DiffStat::default());
}

#[test]
fn not_a_repo_is_zeros() {
    let t = TempDir::new("stat-none");
    std::fs::write(t.0.join("f.txt"), "x\n").unwrap();
    assert_eq!(changes(&t.0), DiffStat::default());
    assert!(!is_worktree(&t.0));
}


#[test]
fn a_nested_repo_is_not_a_change() {
    if !git_available() {
        eprintln!("git not available: skipping");
        return;
    }
    let t = TempDir::new("stat-nested");
    let repo = t.0.join("repo");
    init_repo(&repo);
    // What Claude Code's own worktrees look like: a repo inside the repo, untracked.
    init_repo(&repo.join(".claude/worktrees/wt"));
    assert_eq!(changes(&repo), DiffStat::default());
}
