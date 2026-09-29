use super::*;
use crate::testutil::{init_repo, TempDir};

#[test]
fn run_reports_success_and_captures_stdout() {
    let dir = TempDir::new("git-run-ok");
    init_repo(&dir.0);
    let out = run(&dir.0, &["rev-parse", "HEAD"]).expect("run should not error");
    assert!(out.ok);
    assert_eq!(out.stdout.trim().len(), 40);
    assert!(out.stderr.is_empty());
}

#[test]
fn run_reports_failure_and_captures_stderr() {
    let dir = TempDir::new("git-run-fail");
    init_repo(&dir.0);
    let out = run(&dir.0, &["show", "not-a-real-ref"]).expect("run should not error");
    assert!(!out.ok);
    assert!(!out.stderr.trim().is_empty());
}

/// A pipe's kernel buffer is usually 64 KiB: this exercises draining stdout past that size
/// while git is still writing, the exact scenario the old 2-thread implementation (and this
/// one's single-thread `poll(2)` loop) exists to avoid deadlocking on.
#[test]
fn run_drains_large_stdout_without_deadlocking() {
    let dir = TempDir::new("git-run-large");
    init_repo(&dir.0);
    let big = "x".repeat(300_000);
    std::fs::write(dir.0.join("big.txt"), &big).unwrap();
    assert!(run(&dir.0, &["add", "big.txt"]).unwrap().ok);
    assert!(run(&dir.0, &["commit", "-q", "-m", "big"]).unwrap().ok);
    let out = run(&dir.0, &["show", "HEAD:big.txt"]).expect("run should not error");
    assert!(out.ok);
    assert_eq!(out.stdout.len(), big.len());
}

#[test]
fn toplevel_resolves_the_repo_root() {
    let dir = TempDir::new("git-toplevel");
    init_repo(&dir.0);
    let root = toplevel(&dir.0).unwrap();
    assert_eq!(root, Some(dir.0.clone()));
}

/// Direct test of the timeout/kill path (P06): a fixed, artificially short deadline instead
/// of waiting out the real 30 s `GIT_TIMEOUT`.
#[cfg(unix)]
#[test]
fn wait_for_output_kills_and_times_out_a_hanging_command() {
    use std::process::{Command, Stdio};

    let mut cmd = Command::new("sleep");
    cmd.arg("5").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = cmd.spawn().expect("spawn sleep");
    let start = Instant::now();
    let deadline = start + Duration::from_millis(100);
    let result = wait_for_output(child, deadline, "sleep 5");
    let elapsed = start.elapsed();
    let Err(err) = result else {
        panic!("a 5s sleep past a 100ms deadline must time out");
    };
    assert!(err.contains("didn't finish"), "{err}");
    assert!(elapsed < Duration::from_secs(2), "took {elapsed:?}, should have been killed near the 100ms deadline");
}
