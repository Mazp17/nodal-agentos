//! Diff de un run: `git diff <base>...HEAD` en su carpeta, más lo que haya sin commitear
//! (incluidos archivos nuevos sin trackear), parseado en archivos y hunks.

use std::path::Path;

use serde::Serialize;

use crate::util::git;

/// Tope del patch que se devuelve a la UI (el resto se descarta).
pub const PATCH_MAX: usize = 8 * 1024 * 1024;
const UNTRACKED_MAX: usize = 200;
const UNTRACKED_FILE_MAX: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffFileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffLineKind {
    Context,
    Add,
    Del,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffHunk {
    pub header: String,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub path: String,
    pub old_path: Option<String>,
    pub status: DiffFileStatus,
    pub additions: u32,
    pub deletions: u32,
    pub binary: bool,
    pub hunks: Vec<DiffHunk>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDiff {
    pub base: String,
    pub cwd: String,
    pub includes_working_tree: bool,
    pub files: Vec<FileDiff>,
    pub patch: String,
}

/// Patch crudo de `cwd` contra `base` (con `None`, contra HEAD: solo lo sin commitear).
/// Devuelve `(patch, hay cambios sin commitear)`.
pub fn collect(cwd: &Path, base: Option<&str>) -> Result<(String, bool), String> {
    let from = match base {
        Some(b) => {
            // `base...HEAD` = desde el merge-base; si no hay (historias sin relación), la base.
            let mb = git::run(cwd, &["merge-base", b, "HEAD"])?;
            if mb.ok && !mb.stdout.trim().is_empty() {
                mb.stdout.trim().to_string()
            } else {
                git::ok(cwd, &["rev-parse", "--verify", b])?.trim().to_string()
            }
        }
        None => "HEAD".to_string(),
    };
    let status = git::ok(cwd, &["status", "--porcelain"])?;
    let dirty = !status.trim().is_empty();
    // Sin `..HEAD`: compara el árbol de trabajo contra `from`, que incluye los commits de
    // la rama y lo que no se commiteó.
    let mut patch = git::ok(
        cwd,
        &["-c", "core.quotePath=false", "diff", "-M", "--no-color", "--no-ext-diff", &from, "--"],
    )?;
    if dirty {
        let untracked = git::ok(cwd, &["ls-files", "--others", "--exclude-standard", "-z"])?;
        for file in untracked.split('\0').filter(|f| !f.is_empty()).take(UNTRACKED_MAX) {
            if std::fs::metadata(cwd.join(file)).is_ok_and(|m| m.len() > UNTRACKED_FILE_MAX) {
                patch.push_str(&format!("diff --git a/{file} b/{file}\nnew file mode 100644\nBinary files /dev/null and b/{file} differ\n"));
                continue;
            }
            // `--no-index` sale con 1 cuando hay diferencias: se usa `run`, no `ok`.
            let out = git::run(
                cwd,
                &["-c", "core.quotePath=false", "diff", "--no-index", "--no-color", "--no-ext-diff", "--", "/dev/null", file],
            )?;
            patch.push_str(&out.stdout);
            if patch.len() > PATCH_MAX {
                break;
            }
        }
    }
    if patch.len() > PATCH_MAX {
        let mut cut = PATCH_MAX;
        while !patch.is_char_boundary(cut) {
            cut -= 1;
        }
        patch.truncate(cut);
    }
    Ok((patch, dirty))
}

/// Patch de una rama que no está en un checkout propio (la de un workflow):
/// `git diff <base>...<branch>`, sin árbol de trabajo.
pub fn collect_branch(repo: &Path, base: &str, branch: &str) -> Result<String, String> {
    if branch.starts_with('-') || base.starts_with('-') {
        return Err("Invalid branch name.".into());
    }
    let range = format!("{base}...{branch}");
    let mut patch = git::ok(
        repo,
        &["-c", "core.quotePath=false", "diff", "-M", "--no-color", "--no-ext-diff", &range, "--"],
    )?;
    if patch.len() > PATCH_MAX {
        let mut cut = PATCH_MAX;
        while !patch.is_char_boundary(cut) {
            cut -= 1;
        }
        patch.truncate(cut);
    }
    Ok(patch)
}

fn unquote(p: &str) -> String {
    let p = p.trim_end_matches('\t');
    match p.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        Some(inner) => inner.replace("\\\"", "\"").replace("\\\\", "\\").replace("\\t", "\t"),
        None => p.to_string(),
    }
}

fn strip_side(p: &str) -> Option<String> {
    let p = unquote(p);
    if p == "/dev/null" {
        return None;
    }
    Some(p.strip_prefix("a/").or_else(|| p.strip_prefix("b/")).unwrap_or(&p).to_string())
}

/// `diff --git a/x b/x` → `x` cuando las dos mitades coinciden (caso sin rename).
fn header_path(rest: &str) -> Option<String> {
    let rest = rest.trim();
    let n = rest.len();
    if n % 2 == 1 {
        let (a, b) = (&rest[..n / 2], &rest[n / 2 + 1..]);
        if let (Some(a), Some(b)) = (a.strip_prefix("a/"), b.strip_prefix("b/")) {
            if a == b {
                return Some(a.to_string());
            }
        }
    }
    rest.rsplit_once(" b/").map(|(_, b)| b.to_string())
}

fn parse_range(s: &str) -> (u32, u32) {
    let (start, len) = s.split_once(',').unwrap_or((s, "1"));
    (start.parse().unwrap_or(0), len.parse().unwrap_or(0))
}

fn parse_hunk_header(line: &str) -> Option<DiffHunk> {
    let rest = line.strip_prefix("@@ ")?;
    let (ranges, _) = rest.split_once(" @@")?;
    let mut parts = ranges.split_whitespace();
    let (old_start, old_lines) = parse_range(parts.next()?.strip_prefix('-')?);
    let (new_start, new_lines) = parse_range(parts.next()?.strip_prefix('+')?);
    Some(DiffHunk { header: line.to_string(), old_start, old_lines, new_start, new_lines, lines: Vec::new() })
}

/// Parsea un patch unificado de git en archivos (A/M/D/R, +/−) y hunks.
pub fn parse(patch: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut old_no = 0u32;
    let mut new_no = 0u32;
    for line in patch.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            files.push(FileDiff {
                path: header_path(rest).unwrap_or_default(),
                old_path: None,
                status: DiffFileStatus::Modified,
                additions: 0,
                deletions: 0,
                binary: false,
                hunks: Vec::new(),
            });
            continue;
        }
        let Some(f) = files.last_mut() else { continue };
        if f.hunks.is_empty() {
            // Cabecera del archivo.
            if line.starts_with("new file mode") {
                f.status = DiffFileStatus::Added;
            } else if line.starts_with("deleted file mode") {
                f.status = DiffFileStatus::Deleted;
            } else if let Some(p) = line.strip_prefix("rename from ") {
                f.status = DiffFileStatus::Renamed;
                f.old_path = Some(unquote(p));
            } else if let Some(p) = line.strip_prefix("rename to ") {
                f.status = DiffFileStatus::Renamed;
                f.path = unquote(p);
            } else if let Some(p) = line.strip_prefix("--- ") {
                if strip_side(p).is_none() {
                    f.status = DiffFileStatus::Added;
                }
            } else if let Some(p) = line.strip_prefix("+++ ") {
                match strip_side(p) {
                    Some(path) => f.path = path,
                    None => f.status = DiffFileStatus::Deleted,
                }
            } else if line.starts_with("Binary files ") || line == "GIT binary patch" {
                f.binary = true;
            }
        }
        if line.starts_with("@@ ") {
            if let Some(h) = parse_hunk_header(line) {
                old_no = h.old_start;
                new_no = h.new_start;
                f.hunks.push(h);
            }
            continue;
        }
        let Some(h) = f.hunks.last_mut() else { continue };
        let (kind, text) = match line.chars().next() {
            Some('+') => (DiffLineKind::Add, &line[1..]),
            Some('-') => (DiffLineKind::Del, &line[1..]),
            Some(' ') => (DiffLineKind::Context, &line[1..]),
            // `\ No newline at end of file` y líneas vacías de contexto recortadas.
            Some('\\') => continue,
            None => (DiffLineKind::Context, ""),
            _ => continue,
        };
        let (o, n) = match kind {
            DiffLineKind::Add => {
                f.additions += 1;
                new_no += 1;
                (None, Some(new_no - 1))
            }
            DiffLineKind::Del => {
                f.deletions += 1;
                old_no += 1;
                (Some(old_no - 1), None)
            }
            DiffLineKind::Context => {
                old_no += 1;
                new_no += 1;
                (Some(old_no - 1), Some(new_no - 1))
            }
        };
        h.lines.push(DiffLine { kind, text: text.to_string(), old_no: o, new_no: n });
    }
    files
}

#[cfg(test)]
mod tests {
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
        assert_eq!(files[1].hunks[0].lines.len(), 2, "sin la línea `\\ No newline`");
        assert!(parse("").is_empty());
    }

    #[test]
    fn diff_of_a_worktree_branch_includes_uncommitted_and_untracked() {
        if !git_available() {
            eprintln!("git no disponible: se saltea");
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
        let files = parse(&patch);
        assert_eq!(files.len(), 1);
        assert_eq!((files[0].additions, files[0].deletions), (1, 0));
        // La misma rama vista desde afuera (como la de un workflow).
        assert_eq!(parse(&collect_branch(&repo, "main", "feature").unwrap()).len(), 1);
        assert!(collect_branch(&repo, "main", "--output=x").is_err());

        std::fs::write(repo.join("README.md"), "# demo\ncommitted\nwip\n").unwrap();
        std::fs::write(repo.join("new file.txt"), "hola\n").unwrap();
        let (patch, dirty) = collect(&repo, Some("main")).unwrap();
        assert!(dirty);
        let files = parse(&patch);
        let paths: Vec<(&str, DiffFileStatus, u32)> = files.iter().map(|f| (f.path.as_str(), f.status, f.additions)).collect();
        assert_eq!(paths, [("README.md", DiffFileStatus::Modified, 2), ("new file.txt", DiffFileStatus::Added, 1)]);
        // Sin base (in_place): solo lo sin commitear.
        let (patch, _) = collect(&repo, None).unwrap();
        let files = parse(&patch);
        assert_eq!(files[0].additions, 1);
        assert!(collect(&repo, Some("no-such-branch")).is_err());
    }
}
