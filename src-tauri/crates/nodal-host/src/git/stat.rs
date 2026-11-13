//! Cheap "N files changed +A -D" for a checkout: `git diff HEAD --numstat` plus the untracked,
//! non-ignored files (their lines counted from disk). No patch is ever built.

use std::path::Path;

use nodal_domain::model::DiffStat;

use super::{ok, toplevel};

/// Untracked files above this size count as a file with 0 lines.
const UNTRACKED_FILE_MAX: u64 = 1024 * 1024;
/// Untracked files whose lines are counted; the rest only count as files.
const UNTRACKED_COUNTED_MAX: usize = 200;

/// A linked worktree has `.git` as a file (pointing at the main repo's gitdir).
pub fn is_worktree(dir: &Path) -> bool {
    matches!(toplevel(dir), Ok(Some(top)) if top.join(".git").is_file())
}

/// Zeros when `dir` isn't a git repo.
pub fn changes(dir: &Path) -> DiffStat {
    // No HEAD yet (fresh repo): the tracked part just fails and only untracked files count.
    let mut stat = ok(dir, &["diff", "HEAD", "--numstat"])
        .map(|out| parse_numstat(&out))
        .unwrap_or_default();
    let Ok(out) = ok(dir, &["ls-files", "--others", "--exclude-standard", "-z"]) else {
        return stat;
    };
    // A trailing `/` is a nested repo (e.g. a worktree inside the repo): not a change here.
    for (i, name) in out.split('\0').filter(|s| !s.is_empty() && !s.ends_with('/')).enumerate() {
        stat.files += 1;
        if i < UNTRACKED_COUNTED_MAX {
            stat.additions += count_lines(&dir.join(name));
        }
    }
    stat
}

/// `added\tremoved\tpath` rows; binary files show `-\t-` and count as a file with 0/0.
pub fn parse_numstat(out: &str) -> DiffStat {
    let mut stat = DiffStat::default();
    for line in out.lines() {
        let mut cols = line.splitn(3, '\t');
        let (Some(a), Some(d), Some(_)) = (cols.next(), cols.next(), cols.next()) else { continue };
        stat.files += 1;
        stat.additions += a.parse::<u32>().unwrap_or(0);
        stat.deletions += d.parse::<u32>().unwrap_or(0);
    }
    stat
}

/// Lines of a regular, small, text file; 0 for anything else (symlink, big, binary, unreadable).
fn count_lines(path: &Path) -> u32 {
    let Ok(meta) = std::fs::symlink_metadata(path) else { return 0 };
    if !meta.is_file() || meta.len() > UNTRACKED_FILE_MAX {
        return 0;
    }
    let Ok(bytes) = std::fs::read(path) else { return 0 };
    if bytes.contains(&0) {
        return 0;
    }
    let newlines = bytes.iter().filter(|&&b| b == b'\n').count();
    let unterminated = bytes.last().is_some_and(|&b| b != b'\n');
    (newlines + usize::from(unterminated)) as u32
}

#[cfg(test)]
mod tests;
