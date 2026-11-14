//! Adapters: thin `nodal_domain::ports` implementations that delegate to today's host
//! functions verbatim. This is glue, not logic — the behavior lives in the modules these
//! call into.

use std::io;
use std::path::{Path, PathBuf};

use nodal_domain::diff::CommitInfo;
use nodal_domain::error::HostError;
use nodal_domain::execution::worktree::{MergeOutcome, WorktreeStatus};
use nodal_domain::model::chat::ClaudeDefaults;
use nodal_domain::model::claude::{SubagentRef, Transcript};
use nodal_domain::model::executors::AgentDef;
use nodal_domain::model::{DiffStat, DirEntry, RepoFile, RepoWorktree, TaskPr, WorktreeRef};
use nodal_domain::ports::{ClaudeConfig, Clock, Git, LocalFs, PlanFiles, SessionFiles};

use crate::claude::{catalog, fs, settings};
use crate::git;
use crate::{paths, plans, repo};

/// `util::now_ms`.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
    }
}

/// `~/.claude/projects` (`nodal_domain::ports::SessionFiles`).
pub struct HostSessionFiles;

impl SessionFiles for HostSessionFiles {
    fn find_session_jsonl(&self, projects: &Path, cwd: &str, session_id: &str) -> Option<PathBuf> {
        fs::paths::find_session_jsonl(projects, cwd, session_id)
    }

    fn session_location(&self, path: &Path) -> (Option<String>, Option<String>) {
        crate::claude::session_location::session_location(path)
    }

    fn read_session_transcript(
        &self,
        path: &Path,
        id: &str,
        label: Option<String>,
        model: Option<String>,
        limit: usize,
    ) -> Result<Option<Transcript>, HostError> {
        Ok(fs::transcript::read_session_transcript(path, id, label, model, limit)?)
    }

    fn session_title(&self, path: &Path) -> Option<String> {
        fs::transcript::session_title(path)
    }

    fn last_turn_tools(&self, path: &Path) -> Vec<(String, serde_json::Value)> {
        fs::transcript::last_turn_tools(path)
    }

    fn rewind_point(&self, path: &Path, user_uuid: &str) -> Result<Option<String>, HostError> {
        Ok(fs::transcript::rewind_point(path, user_uuid)?)
    }

    fn list_subagents(&self, session_path: &Path) -> Vec<SubagentRef> {
        fs::transcript::list_subagents(session_path)
    }

    fn read_subagent_transcript(
        &self,
        session_path: &Path,
        agent_id: &str,
        limit: usize,
    ) -> Result<Option<Transcript>, HostError> {
        Ok(fs::transcript::read_subagent_transcript(session_path, agent_id, limit)?)
    }
}

/// Claude Code config on disk (`nodal_domain::ports::ClaudeConfig`).
pub struct HostClaudeConfig;

impl ClaudeConfig for HostClaudeConfig {
    fn list_agents(&self, claude_dir: Option<&Path>, repo: Option<&Path>) -> Vec<AgentDef> {
        catalog::list_agents(claude_dir, repo)
    }

    fn claude_defaults(&self, claude_dir: Option<&Path>, repo: Option<&Path>) -> ClaudeDefaults {
        settings::read(claude_dir, repo)
    }
}

/// git with a timeout (`nodal_domain::ports::Git`).
pub struct HostGit;

impl Git for HostGit {
    fn require_git_root(&self, path: &str) -> Result<PathBuf, HostError> {
        Ok(paths::require_git_root(path)?)
    }

    fn ensure_worktree(
        &self,
        repo: &Path,
        dir: &Path,
        branch: &str,
        existing: Option<&WorktreeRef>,
    ) -> Result<WorktreeRef, HostError> {
        Ok(git::worktree::ensure(repo, dir, branch, existing)?)
    }

    fn cleanup_worktree(&self, repo: &Path, wt: &WorktreeRef) -> Result<(), HostError> {
        Ok(git::worktree::cleanup(repo, wt)?)
    }

    fn worktree_status(&self, repo: &Path, wt: Option<&WorktreeRef>) -> Result<WorktreeStatus, HostError> {
        Ok(git::worktree::status(repo, wt)?)
    }

    fn current_base(&self, repo: &Path) -> Result<String, HostError> {
        Ok(git::worktree::current_base(repo)?)
    }

    fn diff(&self, cwd: &Path, base: Option<&str>) -> Result<(String, bool), HostError> {
        Ok(git::diff::collect(cwd, base)?)
    }

    fn diff_branch(&self, repo: &Path, base: &str, branch: &str) -> Result<String, HostError> {
        Ok(git::diff::collect_branch(repo, base, branch)?)
    }

    fn commits(&self, dir: &Path, base: &str, head: &str) -> Result<Vec<CommitInfo>, HostError> {
        Ok(git::diff::commits(dir, base, head)?)
    }

    fn log(&self, dir: &Path, base: Option<&str>) -> Result<Vec<CommitInfo>, HostError> {
        Ok(git::diff::log(dir, base)?)
    }

    fn show_commit(&self, dir: &Path, sha: &str) -> Result<String, HostError> {
        Ok(git::diff::show(dir, sha)?)
    }

    fn current_branch(&self, dir: &Path) -> Option<String> {
        git::diff::current_branch(dir)
    }

    fn list_dir(&self, repo: &Path, rel: &str) -> Result<Vec<DirEntry>, HostError> {
        Ok(git::ls::list_dir(repo, rel)?)
    }

    fn list_files(&self, repo: &Path) -> Result<Vec<String>, HostError> {
        Ok(git::ls::list_files(repo)?)
    }

    fn read_file(&self, repo: &Path, rel: &str) -> Result<RepoFile, HostError> {
        Ok(git::ls::read_file(repo, rel)?)
    }

    fn write_file(&self, repo: &Path, rel: &str, content: &str, version: &str, force: bool) -> Result<RepoFile, HostError> {
        Ok(git::ls::write_file(repo, rel, content, version, force)?)
    }

    fn list_worktrees(&self, repo: &Path) -> Result<Vec<RepoWorktree>, HostError> {
        Ok(git::ls::list_worktrees(repo)?)
    }

    fn revert_file(&self, root: &Path, rel: &str) -> Result<(), HostError> {
        Ok(git::ls::revert_file(root, rel)?)
    }

    fn is_worktree(&self, dir: &Path) -> bool {
        git::stat::is_worktree(dir)
    }

    fn worktree_added_ms(&self, dir: &Path) -> Option<i64> {
        let t = std::fs::metadata(dir.join(".git")).and_then(|m| m.modified()).ok()?;
        Some(t.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64)
    }

    fn changes(&self, dir: &Path) -> DiffStat {
        git::stat::changes(dir)
    }

    fn merge(&self, repo: &Path, wt: &WorktreeRef, message: &str, squash: bool) -> Result<MergeOutcome, HostError> {
        Ok(git::merge::merge(repo, wt, message, squash)?)
    }

    fn push_base(&self, repo: &Path, base: &str) -> Result<String, HostError> {
        Ok(git::merge::push_base(repo, base)?)
    }

    fn commit_all(&self, dir: &Path, message: &str, paths: &[String]) -> Result<CommitInfo, HostError> {
        Ok(git::pr::commit_all(dir, message, paths)?)
    }

    fn default_branch(&self, dir: &Path) -> Option<String> {
        git::pr::default_branch(dir)
    }

    fn open_pr(&self, dir: &Path, base: &str, branch: &str, title: &str, body: &str) -> Result<String, HostError> {
        Ok(git::pr::open_pr(dir, base, branch, title, body)?)
    }

    fn pr_for_branch(&self, dir: &Path, branch: &str) -> Result<Option<TaskPr>, HostError> {
        Ok(git::pr::pr_for_branch(dir, branch)?)
    }
}

/// Files Nodal owns plus plan files inside repos (`nodal_domain::ports::PlanFiles`).
pub struct HostPlanFiles;

impl PlanFiles for HostPlanFiles {
    fn is_file(&self, path: &Path) -> bool {
        plans::is_file(path)
    }

    fn read_plan(&self, path: &Path) -> Result<String, HostError> {
        Ok(plans::read_plan(path)?)
    }

    fn validate_plan_file(&self, repo: &Path, path: &str) -> Result<PathBuf, HostError> {
        Ok(plans::plan_file(repo, path)?)
    }

    fn write_atomic(&self, path: &Path, contents: &[u8]) -> Result<(), HostError> {
        Ok(plans::write_atomic(path, contents)?)
    }

    fn write_plan(&self, path: &Path, content: &str) -> io::Result<bool> {
        plans::write_plan(path, content)
    }

    fn remove_dir_all(&self, dir: &Path) -> io::Result<()> {
        plans::remove_dir_all(dir)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        plans::remove_file(path)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        plans::rename(from, to)
    }
}

/// Read-only probing of user paths (`nodal_domain::ports::LocalFs`).
pub struct HostLocalFs;

impl LocalFs for HostLocalFs {
    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        dunce::canonicalize(path)
    }

    fn canonical_dir(&self, path: &str) -> Result<PathBuf, HostError> {
        Ok(paths::canonical_dir(path)?)
    }

    fn expand_home(&self, path: &str) -> PathBuf {
        paths::expand_home(path)
    }

    fn test_commands(&self, cwd: &Path) -> Vec<String> {
        repo::test_commands(cwd)
    }

    fn nodal_mcp_bin(&self) -> Option<PathBuf> {
        repo::nodal_mcp_bin()
    }
}
