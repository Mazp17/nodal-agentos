use super::*;
use crate::testutil::{git_available, init_repo, TempDir};
use nodal_domain::diff::{parse, DiffFileStatus};

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

#[test]
fn collect_reuses_the_cached_patch_without_recomputing() {
    if !git_available() {
        eprintln!("git not available: skipping");
        return;
    }
    let t = TempDir::new("diff-cache");
    let repo = t.0.join("repo");
    init_repo(&repo);
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git").arg("-C").arg(&repo).args(args).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    };
    git(&["checkout", "-q", "-b", "feature"]);
    std::fs::write(repo.join("a.txt"), "hello\n").unwrap();
    git(&["add", "a.txt"]);
    git(&["commit", "-qm", "add a"]);

    let (p1, d1) = collect(&repo, Some("main")).unwrap();
    assert!(!d1);
    assert_eq!(parse(&p1).len(), 1);

    // Renaming `main` away doesn't touch HEAD or the working tree, so the cache key (HEAD +
    // status hash) stays identical: a real recompute would fail resolving `main` in
    // `merge-base`/`rev-parse`, so a successful, identical result here proves the cache hit.
    git(&["branch", "-m", "main", "gone"]);
    let (p2, d2) = collect(&repo, Some("main")).unwrap();
    assert_eq!(p1, p2);
    assert_eq!(d1, d2);

    // A new untracked file changes `git status`'s output: the cache is invalidated, and this
    // time the stale `main` really is resolved and fails.
    std::fs::write(repo.join("b.txt"), "world\n").unwrap();
    assert!(collect(&repo, Some("main")).is_err());
}

#[test]
fn run_diff_stays_within_its_process_budget_with_many_new_files() {
    if !git_available() {
        eprintln!("git not available: skipping");
        return;
    }
    let t = TempDir::new("diff-cap");
    let repo = t.0.join("repo");
    init_repo(&repo);
    for i in 0..60 {
        std::fs::write(repo.join(format!("new-{i}.txt")), format!("line {i}\n")).unwrap();
    }

    // `collect` would return `Err` (budget exhausted) if it ever spawned one git process per
    // new file again: a successful result here is itself the process-cap assertion.
    let (patch, dirty) = collect(&repo, Some("main")).unwrap();
    let log = commits(&repo, "main", "HEAD").unwrap();
    let branch = current_branch(&repo);

    assert!(dirty);
    assert!(log.is_empty());
    assert_eq!(branch.as_deref(), Some("main"));
    let files = parse(&patch);
    assert_eq!(files.len(), 60);
    assert!(files.iter().all(|f| f.status == DiffFileStatus::Added && f.additions == 1));
}

#[test]
fn process_budget_fails_closed_once_exhausted() {
    let mut budget = ProcessBudget(2);
    assert!(budget.spend("a").is_ok());
    assert!(budget.spend("b").is_ok());
    assert!(budget.spend("c").is_err());
}

#[test]
fn new_files_are_synthesized_without_git_no_index() {
    if !git_available() {
        eprintln!("git not available: skipping");
        return;
    }
    let t = TempDir::new("diff-synth");
    let repo = t.0.join("repo");
    init_repo(&repo);
    std::fs::write(repo.join("empty.txt"), "").unwrap();
    std::fs::write(repo.join("no-newline.txt"), "line1\nline2").unwrap();
    std::fs::write(repo.join("binary.bin"), [0u8, 1, 2, 3, 0, 5]).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let exe = repo.join("script.sh");
        std::fs::write(&exe, "#!/bin/sh\necho hi\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let (patch, dirty) = collect(&repo, None).unwrap();
    assert!(dirty);
    let files = parse(&patch);
    let by_path = |p: &str| files.iter().find(|f| f.path == p).unwrap();

    let empty = by_path("empty.txt");
    assert_eq!(empty.status, DiffFileStatus::Added);
    assert_eq!(empty.additions, 0);
    assert!(!empty.binary);

    let nn = by_path("no-newline.txt");
    assert_eq!(nn.additions, 2);
    assert_eq!(nn.hunks[0].lines.last().unwrap().text, "line2");

    assert!(by_path("binary.bin").binary);

    #[cfg(unix)]
    assert_eq!(by_path("script.sh").additions, 2);
}

#[test]
fn parse_status_reads_head_and_untracked_files() {
    let raw = [
        "# branch.oid abc123",
        "# branch.head main",
        "1 .M N... 100644 100644 100644 h h README.md",
        "? root.txt",
        "? sub/new.txt",
        "",
    ]
    .join("\0");
    let snap = parse_status(&raw);
    assert_eq!(snap.head, "abc123");
    assert!(snap.dirty);
    assert_eq!(snap.untracked, vec!["root.txt".to_string(), "sub/new.txt".to_string()]);

    let same = parse_status(&raw);
    assert_eq!(snap.hash, same.hash);
    let changed = raw.replace("root.txt", "root2.txt");
    assert_ne!(snap.hash, parse_status(&changed).hash);
}

#[test]
fn parse_status_skips_the_orig_path_field_of_a_rename() {
    if !git_available() {
        eprintln!("git not available: skipping");
        return;
    }
    let t = TempDir::new("diff-rename-status");
    let repo = t.0.join("repo");
    init_repo(&repo);
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git").arg("-C").arg(&repo).args(args).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    };
    git(&["mv", "README.md", "RENAMED.md"]);
    std::fs::write(repo.join("new.txt"), "hi\n").unwrap();

    let snap = status_snapshot(&repo, &mut ProcessBudget(MAX_GIT_PROCESSES)).unwrap();
    assert!(snap.dirty);
    assert_eq!(snap.untracked, vec!["new.txt".to_string()]);
}

#[test]
fn quote_diff_path_only_quotes_when_needed() {
    assert_eq!(quote_diff_path("plain/path.txt"), "plain/path.txt");
    assert_eq!(quote_diff_path("a\"b"), "\"a\\\"b\"");
    assert_eq!(quote_diff_path("a\\b"), "\"a\\\\b\"");
    assert_eq!(quote_diff_path("a\tb"), "\"a\\tb\"");
}

#[test]
fn looks_binary_detects_a_nul_byte() {
    assert!(!looks_binary(b"hello world"));
    assert!(looks_binary(b"hello\0world"));
}

/// Not a correctness check: prints `collect`'s p50 latency for the S2 scenario (50 new
/// untracked files), cold (worktree changes every call, so the cache always misses — what a
/// "live" run's 5 s poll looks like) and warm (nothing changes — reopening an idle run's
/// diff). Run manually: `cargo test -p nodal-host --release git::diff::tests::measure_run_diff_latency -- --ignored --nocapture`.
#[test]
#[ignore]
fn measure_run_diff_latency() {
    if !git_available() {
        eprintln!("git not available: skipping");
        return;
    }
    const N: usize = 50;
    const ITERS: usize = 30;

    let t = TempDir::new("diff-measure-cold");
    let repo = t.0.join("repo");
    init_repo(&repo);
    for i in 0..N {
        std::fs::write(repo.join(format!("new_{i}.txt")), format!("line {i}\n")).unwrap();
    }
    let mut cold = Vec::with_capacity(ITERS);
    for i in 0..ITERS {
        // A new untracked file each round: `git status`'s output changes, so the cache misses.
        std::fs::write(repo.join(format!("touch_{i}.txt")), "x\n").unwrap();
        let start = std::time::Instant::now();
        collect(&repo, None).unwrap();
        cold.push(start.elapsed());
    }

    let t2 = TempDir::new("diff-measure-warm");
    let repo2 = t2.0.join("repo");
    init_repo(&repo2);
    for i in 0..N {
        std::fs::write(repo2.join(format!("new_{i}.txt")), format!("line {i}\n")).unwrap();
    }
    collect(&repo2, None).unwrap(); // Prime the cache.
    let mut warm = Vec::with_capacity(ITERS);
    for _ in 0..ITERS {
        let start = std::time::Instant::now();
        collect(&repo2, None).unwrap();
        warm.push(start.elapsed());
    }

    cold.sort();
    warm.sort();
    eprintln!(
        "run_diff/{N}-new-files cold(cache miss every call): p50={:?} p95={:?}",
        cold[cold.len() / 2],
        cold[(cold.len() as f64 * 0.95) as usize]
    );
    eprintln!(
        "run_diff/{N}-new-files warm(cache hit): p50={:?} p95={:?}",
        warm[warm.len() / 2],
        warm[(warm.len() as f64 * 0.95) as usize]
    );
}
