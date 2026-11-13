use super::*;

#[test]
fn tail_reads_only_the_end() {
    let tmp = std::env::temp_dir().join(format!("nodal-session-tail-{}.jsonl", std::process::id()));
    let mut s = String::new();
    for i in 0..2000 {
        s.push_str(&format!("{{\"type\":\"user\",\"n\":{i}}}\n"));
    }
    fs::write(&tmp, &s).unwrap();
    let tail = read_tail(&tmp, 256).unwrap();
    assert!(tail.len() <= 256);
    assert!(tail.lines().all(|l| l.starts_with('{') && l.ends_with('}')), "{tail}");
    assert!(tail.contains("\"n\":1999"));
    fs::remove_file(&tmp).unwrap();
}

#[test]
fn takes_the_latest_cwd_and_git_branch() {
    let (cwd, branch) = latest_location(concat!(
        "{\"type\":\"user\",\"cwd\":\"/repo\",\"gitBranch\":\"main\"}\n",
        "{\"type\":\"assistant\",\"cwd\":\"/repo/.claude/worktrees/x\",\"gitBranch\":\"worktree-x\"}\n",
        "{\"type\":\"system\"}\n",
    ));
    assert_eq!(cwd.as_deref(), Some("/repo/.claude/worktrees/x"));
    assert_eq!(branch.as_deref(), Some("worktree-x"));
    assert_eq!(latest_location("").0, None);
}
