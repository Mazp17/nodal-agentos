//! Task worktree naming and status types. Creating, reusing and merging worktrees (git and
//! disk I/O) lives in nodal-host.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::model::Task;

pub const BRANCH_PREFIX: &str = "nodal/";
const SLUG_MAX: usize = 48;

/// Lowercase ASCII, digits and `-` (not repeated, not at the edges).
pub fn slugify(s: &str, max: usize) -> String {
    let mut out = String::new();
    for c in s.chars() {
        let c = match c {
            'á' | 'à' | 'ä' | 'â' | 'Á' | 'À' | 'Ä' | 'Â' => 'a',
            'é' | 'è' | 'ë' | 'ê' | 'É' | 'È' | 'Ë' | 'Ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' | 'Í' | 'Ì' | 'Ï' | 'Î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' | 'Ó' | 'Ò' | 'Ö' | 'Ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' | 'Ú' | 'Ù' | 'Ü' | 'Û' => 'u',
            'ñ' | 'Ñ' => 'n',
            c => c,
        };
        if c.is_ascii_alphanumeric() {
            if out.len() >= max {
                break;
            }
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            if out.len() + 1 >= max {
                break;
            }
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// `pay-1-new-logo-in-the-header`: the task key (unique in the app) plus the title.
pub fn task_slug(project_key: &str, number: i64, title: &str) -> String {
    let key = slugify(&format!("{project_key}-{number}"), 20);
    let rest = slugify(title, SLUG_MAX.saturating_sub(key.len() + 1));
    if rest.is_empty() {
        key
    } else {
        format!("{key}-{rest}")
    }
}

pub fn branch_for(slug: &str) -> String {
    format!("{BRANCH_PREFIX}{slug}")
}

/// Worktree folder: `<root>/<repo>/<slug>` (`root` = `~/.nodal/worktrees`).
pub fn dir_for(root: &Path, repo_name: &str, slug: &str) -> PathBuf {
    let repo = slugify(repo_name, 60);
    root.join(if repo.is_empty() {
        "repo".to_string()
    } else {
        repo
    })
    .join(slug)
}

/// State of a task's worktree, to decide whether cleaning it up is safe.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeStatus {
    /// The folder exists and is a live worktree.
    pub exists: bool,
    pub branch: Option<String>,
    pub base: Option<String>,
    /// Branch commits that aren't in the base.
    pub ahead: u32,
    /// Branch commits that are neither in the base nor in any remote: they would be lost.
    pub unpushed: u32,
    /// Uncommitted changes (including new, non-ignored files).
    pub dirty: bool,
}

/// Why it can't be cleaned up without `force` (`None`: it's safe).
pub fn cleanup_blocker(st: &WorktreeStatus) -> Option<String> {
    let mut why = Vec::new();
    if st.unpushed > 0 {
        let s = if st.unpushed == 1 { "" } else { "s" };
        why.push(format!("{} unpushed commit{s}", st.unpushed));
    }
    if st.dirty {
        why.push("uncommitted changes".to_string());
    }
    (!why.is_empty()).then(|| {
        format!("The worktree has {}: push or commit them first, or clean up with force to discard them.", why.join(" and "))
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MergeOutcome {
    /// `commit`: the base's new tip. `commits`: task commits that landed. `moved`: the base had
    /// moved and was merged into the worktree first.
    Merged {
        commit: String,
        commits: u32,
        squashed: bool,
        moved: bool,
    },
    /// Merging the base into the worktree conflicted in `files`; the merge was aborted.
    Conflict { files: Vec<String> },
}

/// What `merge_worktree` did. `task` is the task after it (Done unless it conflicted).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeReport {
    pub outcome: MergeOutcome,
    pub task: Task,
    /// Remote the base was pushed to (only when asked).
    pub pushed_to: Option<String>,
    pub push_error: Option<String>,
    /// Why the worktree couldn't be cleaned up; the merge itself went through.
    pub cleanup_error: Option<String>,
}

/// Short id of a run (`claude --bg` prints hex). It goes into a command and into AppleScript.
pub fn is_valid_run_id(id: &str) -> bool {
    (4..=64).contains(&id.len())
        && !id.starts_with('-')
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

#[cfg(test)]
mod tests;
