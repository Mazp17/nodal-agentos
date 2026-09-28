//! A run's diff: `git diff <base>...HEAD` in its folder, plus anything uncommitted
//! (including new untracked files), parsed into files and hunks.
//!
//! Types and parsing live in `nodal_domain::diff`.

use std::path::Path;

use nodal_domain::diff::{parse_log, CommitInfo, COMMITS_MAX, PATCH_MAX};

use super::{ok, run};

const UNTRACKED_MAX: usize = 200;
const UNTRACKED_FILE_MAX: u64 = 1024 * 1024;

/// Commits in `head` that aren't in `base` (`git log base..head`).
pub fn commits(dir: &Path, base: &str, head: &str) -> Result<Vec<CommitInfo>, String> {
    if base.starts_with('-') || head.starts_with('-') {
        return Err("Invalid branch name.".into());
    }
    let range = format!("{base}..{head}");
    let max = COMMITS_MAX.to_string();
    let out = ok(dir, &["log", "--no-color", "--format=%H%x1f%h%x1f%an%x1f%at%x1f%s", "-n", &max, &range, "--"])?;
    Ok(parse_log(&out))
}

/// Branch checked out in `dir`; `None` with a detached HEAD.
pub fn current_branch(dir: &Path) -> Option<String> {
    let b = ok(dir, &["rev-parse", "--abbrev-ref", "HEAD"]).ok()?;
    let b = b.trim();
    (!b.is_empty() && b != "HEAD").then(|| b.to_string())
}

/// Raw patch of `cwd` against `base` (with `None`, against HEAD: only uncommitted work).
/// Returns `(patch, has uncommitted changes)`.
pub fn collect(cwd: &Path, base: Option<&str>) -> Result<(String, bool), String> {
    let from = match base {
        Some(b) => {
            // `base...HEAD` = from the merge-base; if there's none (unrelated histories), the base.
            let mb = run(cwd, &["merge-base", b, "HEAD"])?;
            if mb.ok && !mb.stdout.trim().is_empty() {
                mb.stdout.trim().to_string()
            } else {
                ok(cwd, &["rev-parse", "--verify", b])?.trim().to_string()
            }
        }
        None => "HEAD".to_string(),
    };
    let status = ok(cwd, &["status", "--porcelain"])?;
    let dirty = !status.trim().is_empty();
    // Without `..HEAD`: compares the working tree against `from`, which includes the branch's
    // commits and whatever wasn't committed.
    let mut patch = ok(
        cwd,
        &["-c", "core.quotePath=false", "diff", "-M", "--no-color", "--no-ext-diff", &from, "--"],
    )?;
    if dirty {
        let untracked = ok(cwd, &["ls-files", "--others", "--exclude-standard", "-z"])?;
        for file in untracked.split('\0').filter(|f| !f.is_empty()).take(UNTRACKED_MAX) {
            if std::fs::metadata(cwd.join(file)).is_ok_and(|m| m.len() > UNTRACKED_FILE_MAX) {
                patch.push_str(&format!("diff --git a/{file} b/{file}\nnew file mode 100644\nBinary files /dev/null and b/{file} differ\n"));
                continue;
            }
            // `--no-index` exits with 1 when there are differences: use `run`, not `ok`.
            let out = run(
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

/// Patch of a branch that isn't in its own checkout (a workflow's):
/// `git diff <base>...<branch>`, without a working tree.
pub fn collect_branch(repo: &Path, base: &str, branch: &str) -> Result<String, String> {
    if branch.starts_with('-') || base.starts_with('-') {
        return Err("Invalid branch name.".into());
    }
    let range = format!("{base}...{branch}");
    let mut patch = ok(
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

#[cfg(test)]
mod tests;
