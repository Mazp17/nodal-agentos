//! Session location on disk.

use std::fs;
use std::path::{Path, PathBuf};

/// `$CLAUDE_CONFIG_DIR` or `~/.claude`.
pub fn claude_config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".claude"))
}

/// Project directory slug: every non-alphanumeric character becomes `-`.
/// Verified against `~/.claude/projects`: `/Users/x/Code/nodal-sandbox` →
/// `-Users-x-Code-nodal-sandbox`, and `/.claude/worktrees` → `--claude-worktrees`.
pub fn project_slug(cwd: &str) -> String {
    cwd.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

/// `<projects>/<slug>/<sessionId>`. If the slug doesn't match (long paths or changed
/// rules), looks for the session in any project.
pub fn find_session_dir(projects: &Path, cwd: &str, session_id: &str) -> Option<PathBuf> {
    let direct = projects.join(project_slug(cwd)).join(session_id);
    if direct.is_dir() {
        return Some(direct);
    }
    fs::read_dir(projects)
        .ok()?
        .flatten()
        .map(|e| e.path().join(session_id))
        .find(|p| p.is_dir())
}

/// `<projects>/<slug(cwd)>/<sid>.jsonl`, or the first one found in another project.
pub fn find_session_jsonl(projects: &Path, cwd: &str, session_id: &str) -> Option<PathBuf> {
    let file = format!("{session_id}.jsonl");
    let direct = projects.join(project_slug(cwd)).join(&file);
    if direct.is_file() {
        return Some(direct);
    }
    fs::read_dir(projects).ok()?.flatten().map(|e| e.path().join(&file)).find(|p| p.is_file())
}

#[cfg(test)]
mod tests;
