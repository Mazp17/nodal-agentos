use super::*;
use crate::testutil::{git_available, init_repo, TempDir};

fn names(v: &[DirEntry]) -> Vec<(&str, bool)> {
    v.iter().map(|e| (e.name.as_str(), e.is_dir)).collect()
}

#[test]
fn lists_dirs_first_sorted_without_git_or_ignored() {
    if !git_available() {
        eprintln!("git not available: skipping");
        return;
    }
    let t = TempDir::new("ls");
    let repo = t.0.join("repo");
    init_repo(&repo);
    std::fs::write(repo.join(".gitignore"), "*.log\nbuild/\n").unwrap();
    for d in ["src", "Beta", "alpha", "build"] {
        std::fs::create_dir_all(repo.join(d)).unwrap();
    }
    for f in ["b.txt", "A.txt", "x.log"] {
        std::fs::write(repo.join(f), "").unwrap();
    }
    std::fs::write(repo.join("src/main.rs"), "").unwrap();

    let root = list_dir(&repo, "").unwrap();
    assert_eq!(
        names(&root),
        [
            ("alpha", true),
            ("Beta", true),
            ("src", true),
            (".gitignore", false),
            ("A.txt", false),
            ("b.txt", false),
            ("README.md", false),
        ]
    );
    assert_eq!(names(&list_dir(&repo, "src").unwrap()), [("main.rs", false)]);
    assert_eq!(list_dir(&repo, "./src").unwrap().len(), 1);
}

#[test]
fn rejects_absolute_traversal_and_symlink_escape() {
    let t = TempDir::new("ls-escape");
    let repo = t.0.join("repo");
    let outside = t.0.join("outside");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, repo.join("link")).unwrap();

    assert!(list_dir(&repo, outside.to_str().unwrap()).unwrap_err().contains("relative"));
    assert!(list_dir(&repo, "..").unwrap_err().contains("relative"));
    assert!(list_dir(&repo, "a/../..").unwrap_err().contains("relative"));
    #[cfg(unix)]
    assert!(list_dir(&repo, "link").unwrap_err().contains("outside"));
    assert!(list_dir(&repo, "missing").is_err());
}

#[test]
fn read_file_validates_paths_and_detects_binary_and_truncation() {
    let t = TempDir::new("ls-read");
    let repo = t.0.join("repo");
    let outside = t.0.join("outside.txt");
    std::fs::create_dir_all(repo.join("d")).unwrap();
    std::fs::write(&outside, "secret").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, repo.join("link.txt")).unwrap();

    std::fs::write(repo.join("a.txt"), "héllo\n").unwrap();
    let f = read_file(&repo, "a.txt").unwrap();
    assert_eq!((f.content.as_str(), f.binary, f.truncated, f.size), ("héllo\n", false, false, 7));

    assert!(read_file(&repo, "../outside.txt").unwrap_err().contains("relative"));
    assert!(read_file(&repo, outside.to_str().unwrap()).unwrap_err().contains("relative"));
    #[cfg(unix)]
    assert!(read_file(&repo, "link.txt").unwrap_err().contains("outside"));
    assert!(read_file(&repo, "d").unwrap_err().contains("not a regular file"));
    assert!(read_file(&repo, "nope").is_err());

    std::fs::write(repo.join("nul.bin"), [b'a', 0, b'b']).unwrap();
    let f = read_file(&repo, "nul.bin").unwrap();
    assert!(f.binary && f.content.is_empty() && f.size == 3);
    std::fs::write(repo.join("bad.txt"), [0xffu8, 0xfe, b'x']).unwrap();
    assert!(read_file(&repo, "bad.txt").unwrap().binary);

    // 1 MB limit lands in the middle of a 2-byte char: the prefix stays valid text.
    let mut big = "a".repeat(FILE_MAX as usize - 1).into_bytes();
    big.extend("é".as_bytes());
    big.extend(b"tail");
    std::fs::write(repo.join("big.txt"), &big).unwrap();
    let f = read_file(&repo, "big.txt").unwrap();
    assert!(f.truncated && !f.binary && f.size == big.len() as u64);
    assert_eq!(f.content.len(), FILE_MAX as usize - 1);
}

#[test]
fn write_file_round_trips_detects_conflicts_and_refuses_bad_targets() {
    let t = TempDir::new("ls-write");
    let repo = t.0.join("repo");
    let outside = t.0.join("outside.txt");
    std::fs::create_dir_all(repo.join("d")).unwrap();
    std::fs::write(&outside, "secret").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, repo.join("link.txt")).unwrap();
    let file = repo.join("a.sh");
    std::fs::write(&file, "one\r\ntwo\r\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let v1 = read_file(&repo, "a.sh").unwrap();
    // Round trip, bytes exactly as given (CRLF kept), mode kept, no temp file left.
    let saved = write_file(&repo, "a.sh", "three\r\nfour", &v1.version, false).unwrap();
    assert_eq!(saved.content, "three\r\nfour");
    assert_eq!(std::fs::read(&file).unwrap(), b"three\r\nfour");
    assert!(std::fs::read_dir(&repo).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".tmp")));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o755);
    }

    // Stale version -> conflict; force overwrites; the fresh version is accepted.
    assert_ne!(saved.version, v1.version);
    let err = write_file(&repo, "a.sh", "stale", &v1.version, false).unwrap_err();
    assert!(err.starts_with("File changed on disk"), "{err}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "three\r\nfour");
    assert_eq!(write_file(&repo, "a.sh", "forced", &v1.version, true).unwrap().content, "forced");
    let cur = read_file(&repo, "a.sh").unwrap().version;
    assert!(write_file(&repo, "a.sh", "ok", &cur, false).is_ok());

    // Path rules and target kinds.
    assert!(write_file(&repo, "../outside.txt", "x", "", true).unwrap_err().contains("relative"));
    assert!(write_file(&repo, outside.to_str().unwrap(), "x", "", true).unwrap_err().contains("relative"));
    #[cfg(unix)]
    assert!(write_file(&repo, "link.txt", "x", "", true).unwrap_err().contains("outside"));
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "secret");
    assert!(write_file(&repo, "new.txt", "x", "", true).is_err());
    assert!(!repo.join("new.txt").exists());
    assert!(write_file(&repo, "d", "x", "", true).unwrap_err().contains("not a regular file"));

    // Binary, oversize on disk, oversize content.
    std::fs::write(repo.join("b.bin"), [0u8, 1]).unwrap();
    assert!(write_file(&repo, "b.bin", "x", "", true).unwrap_err().contains("can't be edited"));
    std::fs::write(repo.join("big.txt"), "a".repeat(FILE_MAX as usize + 1)).unwrap();
    assert!(write_file(&repo, "big.txt", "x", "", true).unwrap_err().contains("can't be edited"));
    let huge = "a".repeat(WRITE_MAX as usize + 1);
    assert!(write_file(&repo, "a.sh", &huge, "", true).unwrap_err().contains("larger than"));
}

fn git(repo: &Path, args: &[&str]) {
    let out = std::process::Command::new("git").arg("-C").arg(repo).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn porcelain(repo: &Path) -> String {
    let out = std::process::Command::new("git").arg("-C").arg(repo).args(["status", "--porcelain"]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn lists_worktrees_main_first() {
    if !git_available() {
        return;
    }
    let t = TempDir::new("ls-wt");
    let repo = t.0.join("repo");
    init_repo(&repo);
    let wt = t.0.join("wt");
    git(&repo, &["worktree", "add", "-q", "-b", "side", wt.to_str().unwrap()]);
    let gone = t.0.join("gone");
    git(&repo, &["worktree", "add", "-q", "-b", "gone", gone.to_str().unwrap()]);
    std::fs::remove_dir_all(&gone).unwrap();
    let list = list_worktrees(&repo).unwrap();
    assert_eq!(list.len(), 2, "{list:?}");
    assert!(list[0].is_main && list[0].branch.as_deref() == Some("main"));
    assert!(!list[1].is_main && list[1].branch.as_deref() == Some("side"));
    assert!(Path::new(&list[1].path).ends_with("wt"));
}

#[test]
fn revert_restores_tracked_and_removes_new_files() {
    if !git_available() {
        return;
    }
    let t = TempDir::new("ls-revert");
    let repo = t.0.join("repo");
    init_repo(&repo);
    std::fs::write(repo.join("keep.txt"), "orig\n").unwrap();
    std::fs::write(repo.join(".gitignore"), "*.log\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "base"]);

    // Modified (staged and unstaged) and deleted tracked files come back from HEAD.
    std::fs::write(repo.join("keep.txt"), "changed\n").unwrap();
    git(&repo, &["add", "keep.txt"]);
    std::fs::write(repo.join("keep.txt"), "changed again\n").unwrap();
    revert_file(&repo, "keep.txt").unwrap();
    assert_eq!(std::fs::read_to_string(repo.join("keep.txt")).unwrap(), "orig\n");
    std::fs::remove_file(repo.join("README.md")).unwrap();
    revert_file(&repo, "./README.md").unwrap();
    assert!(repo.join("README.md").exists());

    // Untracked and staged-new files are deleted (and unstaged).
    std::fs::write(repo.join("untracked.txt"), "x").unwrap();
    revert_file(&repo, "untracked.txt").unwrap();
    assert!(!repo.join("untracked.txt").exists());
    std::fs::write(repo.join("added.txt"), "x").unwrap();
    git(&repo, &["add", "added.txt"]);
    revert_file(&repo, "added.txt").unwrap();
    assert!(!repo.join("added.txt").exists());

    // A staged rename restores the old path and removes the new one.
    git(&repo, &["mv", "keep.txt", "moved.txt"]);
    revert_file(&repo, "moved.txt").unwrap();
    assert!(repo.join("keep.txt").exists() && !repo.join("moved.txt").exists());
    assert_eq!(porcelain(&repo), "");

    // Clean and ignored files are never touched.
    std::fs::write(repo.join("noise.log"), "x").unwrap();
    assert!(revert_file(&repo, "noise.log").unwrap_err().contains("no uncommitted changes"));
    assert!(revert_file(&repo, "keep.txt").unwrap_err().contains("no uncommitted changes"));
    assert!(repo.join("noise.log").exists());
}

#[test]
fn revert_rejects_bad_paths() {
    let t = TempDir::new("ls-revert-bad");
    let repo = t.0.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let outside = t.0.join("outside.txt");
    std::fs::write(&outside, "keep").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&t.0, repo.join("up")).unwrap();
    assert!(revert_file(&repo, "../outside.txt").unwrap_err().contains("relative"));
    assert!(revert_file(&repo, outside.to_str().unwrap()).unwrap_err().contains("relative"));
    assert!(revert_file(&repo, ".git/config").unwrap_err().contains(".git"));
    assert!(revert_file(&repo, "").is_err());
    #[cfg(unix)]
    assert!(revert_file(&repo, "up/outside.txt").unwrap_err().contains("outside"));
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "keep");
}

#[test]
fn version_changes_with_same_size_edits() {
    let t = TempDir::new("ls-version");
    let repo = t.0.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("a.txt"), "aaaa").unwrap();
    let v1 = read_file(&repo, "a.txt").unwrap().version;
    assert_eq!(v1.split('-').count(), 3);
    // Same size, mtime put back: only the content hash tells them apart.
    let mtime = std::fs::metadata(repo.join("a.txt")).unwrap().modified().unwrap();
    std::fs::write(repo.join("a.txt"), "bbbb").unwrap();
    std::fs::File::options().write(true).open(repo.join("a.txt")).unwrap().set_modified(mtime).unwrap();
    assert_ne!(read_file(&repo, "a.txt").unwrap().version, v1);
}
