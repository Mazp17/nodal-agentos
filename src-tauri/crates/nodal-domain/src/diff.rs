//! Diff types and unified-patch parsing. Collecting the raw patch from git (`git diff`,
//! `git log`) lives in nodal-host.

use serde::Serialize;

/// Cap on the patch returned to the UI (the rest is discarded).
pub const PATCH_MAX: usize = 8 * 1024 * 1024;

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
    /// The run's branch (the worktree's, the workflow's or the checkout's). `None` if HEAD
    /// is detached or couldn't be read.
    pub branch: Option<String>,
    /// Branch commits that aren't in `base`, newest to oldest (up to `COMMITS_MAX`). Empty
    /// without a base (in place).
    pub commits: Vec<CommitInfo>,
    /// The run is still active: the diff may change.
    pub live: bool,
    pub cwd: String,
    pub includes_working_tree: bool,
    pub files: Vec<FileDiff>,
    pub patch: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitInfo {
    pub sha: String,
    pub short_sha: String,
    pub subject: String,
    pub author: String,
    /// Author date, epoch ms.
    pub at: i64,
}

pub const COMMITS_MAX: usize = 200;
const SEP: char = '\u{1f}';

/// Parses `git log --format=%H%x1f%h%x1f%an%x1f%at%x1f%s`.
pub fn parse_log(text: &str) -> Vec<CommitInfo> {
    text.lines()
        .filter_map(|l| {
            let mut p = l.splitn(5, SEP);
            let (sha, short_sha, author, at, subject) =
                (p.next()?, p.next()?, p.next()?, p.next()?, p.next()?);
            Some(CommitInfo {
                sha: sha.into(),
                short_sha: short_sha.into(),
                subject: subject.into(),
                author: author.into(),
                at: at.trim().parse::<i64>().ok()? * 1000,
            })
        })
        .collect()
}

fn unquote(p: &str) -> String {
    let p = p.trim_end_matches('\t');
    match p.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        Some(inner) => inner
            .replace("\\\"", "\"")
            .replace("\\\\", "\\")
            .replace("\\t", "\t"),
        None => p.to_string(),
    }
}

fn strip_side(p: &str) -> Option<String> {
    let p = unquote(p);
    if p == "/dev/null" {
        return None;
    }
    Some(
        p.strip_prefix("a/")
            .or_else(|| p.strip_prefix("b/"))
            .unwrap_or(&p)
            .to_string(),
    )
}

/// `diff --git a/x b/x` → `x` when both halves match (the no-rename case).
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
    Some(DiffHunk {
        header: line.to_string(),
        old_start,
        old_lines,
        new_start,
        new_lines,
        lines: Vec::new(),
    })
}

/// Parses a git unified patch into files (A/M/D/R, +/−) and hunks.
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
            // File header.
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
        let Some(h) = f.hunks.last_mut() else {
            continue;
        };
        let (kind, text) = match line.chars().next() {
            Some('+') => (DiffLineKind::Add, &line[1..]),
            Some('-') => (DiffLineKind::Del, &line[1..]),
            Some(' ') => (DiffLineKind::Context, &line[1..]),
            // `\ No newline at end of file` and trimmed empty context lines.
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
        h.lines.push(DiffLine {
            kind,
            text: text.to_string(),
            old_no: o,
            new_no: n,
        });
    }
    files
}

#[cfg(test)]
mod tests;
