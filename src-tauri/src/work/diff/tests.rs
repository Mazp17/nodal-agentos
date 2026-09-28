use super::*;
use crate::util::paths::tests::{git_available, init_repo, TempDir};

const SAMPLE: &str = "diff --git a/src/app.ts b/src/app.ts
index 1111111..2222222 100644
--- a/src/app.ts
+++ b/src/app.ts
@@ -1,3 +1,4 @@ export function main() {
 const a = 1;
-const b = 2;
+const b = 3;
+const c = 4;
 export {};
diff --git a/docs/new.md b/docs/new.md
new file mode 100644
index 0000000..3333333
--- /dev/null
+++ b/docs/new.md
@@ -0,0 +1,2 @@
+# New
+text
\\ No newline at end of file
diff --git a/old.txt b/old.txt
deleted file mode 100644
index 4444444..0000000
--- a/old.txt
+++ /dev/null
@@ -1 +0,0 @@
-bye
diff --git a/a b.txt b/c d.txt
similarity index 90%
rename from a b.txt
rename to c d.txt
index 5555555..6666666 100644
--- a/a b.txt
+++ b/c d.txt
@@ -1 +1 @@
-x
+y
diff --git a/logo.png b/logo.png
new file mode 100644
index 0000000..7777777
Binary files /dev/null and b/logo.png differ
";

#[test]
fn parses_statuses_counts_and_line_numbers() {
    let files = parse(SAMPLE);
    let summary: Vec<(&str, DiffFileStatus, u32, u32, bool)> =
        files.iter().map(|f| (f.path.as_str(), f.status, f.additions, f.deletions, f.binary)).collect();
    assert_eq!(
        summary,
        [
            ("src/app.ts", DiffFileStatus::Modified, 2, 1, false),
            ("docs/new.md", DiffFileStatus::Added, 2, 0, false),
            ("old.txt", DiffFileStatus::Deleted, 0, 1, false),
            ("c d.txt", DiffFileStatus::Renamed, 1, 1, false),
            ("logo.png", DiffFileStatus::Added, 0, 0, true),
        ]
    );
    assert_eq!(files[3].old_path.as_deref(), Some("a b.txt"));
    let h = &files[0].hunks[0];
    assert_eq!((h.old_start, h.old_lines, h.new_start, h.new_lines), (1, 3, 1, 4));
    assert_eq!(h.header, "@@ -1,3 +1,4 @@ export function main() {");
    let nums: Vec<(DiffLineKind, Option<u32>, Option<u32>)> = h.lines.iter().map(|l| (l.kind, l.old_no, l.new_no)).collect();
    assert_eq!(
        nums,
        [
            (DiffLineKind::Context, Some(1), Some(1)),
            (DiffLineKind::Del, Some(2), None),
            (DiffLineKind::Add, None, Some(2)),
            (DiffLineKind::Add, None, Some(3)),
            (DiffLineKind::Context, Some(3), Some(4)),
        ]
    );
    assert_eq!(files[1].hunks[0].lines.len(), 2, "without the `\\ No newline` line");
    assert!(parse("").is_empty());
}

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
        let out = std::process::Command::new("git").arg("-C").arg(&repo).args(args).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    };
    git(&["checkout", "-q", "-b", "feature"]);
    std::fs::write(repo.join("README.md"), "# demo\ncommitted\n").unwrap();
    git(&["commit", "-qam", "change"]);
    let (patch, dirty) = collect(&repo, Some("main")).unwrap();
    assert!(!dirty);
    assert_eq!(current_branch(&repo).as_deref(), Some("feature"));
    let log = commits(&repo, "main", "HEAD").unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!((log[0].subject.as_str(), log[0].author.as_str()), ("change", "Test"));
    assert!(log[0].sha.starts_with(&log[0].short_sha) && log[0].at > 0);
    assert_eq!(commits(&repo, "main", "feature").unwrap(), log);
    assert!(commits(&repo, "feature", "HEAD").unwrap().is_empty());
    assert!(commits(&repo, "--all", "HEAD").is_err());
    let files = parse(&patch);
    assert_eq!(files.len(), 1);
    assert_eq!((files[0].additions, files[0].deletions), (1, 0));
    // The same branch seen from outside (like a workflow's).
    assert_eq!(parse(&collect_branch(&repo, "main", "feature").unwrap()).len(), 1);
    assert!(collect_branch(&repo, "main", "--output=x").is_err());

    std::fs::write(repo.join("README.md"), "# demo\ncommitted\nwip\n").unwrap();
    std::fs::write(repo.join("new file.txt"), "hello\n").unwrap();
    let (patch, dirty) = collect(&repo, Some("main")).unwrap();
    assert!(dirty);
    let files = parse(&patch);
    let paths: Vec<(&str, DiffFileStatus, u32)> = files.iter().map(|f| (f.path.as_str(), f.status, f.additions)).collect();
    assert_eq!(paths, [("README.md", DiffFileStatus::Modified, 2), ("new file.txt", DiffFileStatus::Added, 1)]);
    // No base (in_place): only uncommitted work.
    let (patch, _) = collect(&repo, None).unwrap();
    let files = parse(&patch);
    assert_eq!(files[0].additions, 1);
    assert!(collect(&repo, Some("no-such-branch")).is_err());
}

#[test]
fn parse_log_tolerates_bad_lines() {
    let text = "aaaa\u{1f}aa\u{1f}Ana\u{1f}1700000000\u{1f}fix: a\u{1f}b\nbroken\nbbbb\u{1f}bb\u{1f}Ana\u{1f}x\u{1f}s\n";
    let log = parse_log(text);
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].subject, "fix: a\u{1f}b", "the subject may contain the separator");
    assert_eq!(log[0].at, 1_700_000_000_000);
}
