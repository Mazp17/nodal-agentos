//! Adapters: thin `nodal_domain::ports` implementations that delegate to today's host
//! functions verbatim. This is glue, not logic — the behavior lives in the modules these
//! call into.

use std::io;
use std::path::{Path, PathBuf};

use tokio::runtime::Handle;

use nodal_domain::diff::CommitInfo;
use nodal_domain::error::HostError;
use nodal_domain::execution::worktree::{MergeOutcome, WorktreeStatus};
use nodal_domain::model::activity::{AgentSession, AppRuns, ExternalSessions};
use nodal_domain::model::chat::ClaudeDefaults;
use nodal_domain::model::claude::{
    ExtraFlags, LaunchBlocker, LaunchError, RunDetail, RunRef, RunSummary, SessionReadout, Transcript,
};
use nodal_domain::model::executors::{AgentDef, ExecutorInfo};
use nodal_domain::model::{LaunchOptions, WorktreeRef};
use nodal_domain::ports::{BoxFut, ClaudeCli, ClaudeConfig, Clock, Git, LocalFs, PlanFiles, SessionFiles};

use crate::claude::live::AgentsRaw;
use crate::claude::{activity, catalog, cli, fs, settings};
use crate::git;
use crate::{paths, plans, repo};

/// `util::now_ms`.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
    }
}

/// The `claude` CLI (`nodal_domain::ports::ClaudeCli`). `agents`: the single-flight cache
/// (P01) `list_sessions`/`list_agent_sessions` share instead of each spawning `claude agents`;
/// `launch_bg`/`stop` invalidate it, since either can change the live set.
pub struct HostClaudeCli {
    rt: Handle,
    agents: AgentsRaw,
}

impl HostClaudeCli {
    pub fn new(rt: Handle) -> Self {
        Self { rt, agents: AgentsRaw::new() }
    }

    /// `claude agents` served by a fake `program` instead of the real `claude`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_program(rt: Handle, program: std::path::PathBuf) -> Self {
        Self { rt, agents: AgentsRaw::with_program(program) }
    }
}

impl ClaudeCli for HostClaudeCli {
    fn list_sessions(&self) -> BoxFut<'_, Result<Vec<RunSummary>, HostError>> {
        Box::pin(async move { Ok(cli::list_runs(&self.agents).await?) })
    }

    fn list_agent_sessions(&self) -> BoxFut<'_, Result<Vec<AgentSession>, HostError>> {
        Box::pin(async move { Ok(activity::list_agents(&self.agents).await?) })
    }

    fn launch_bg<'a>(
        &'a self,
        cwd: String,
        prompt: String,
        opts: &'a LaunchOptions,
        extra: &'a ExtraFlags,
    ) -> BoxFut<'a, Result<RunRef, LaunchError>> {
        Box::pin(async move {
            let r = cli::launch_bg(cwd, prompt, opts, extra, &self.rt).await;
            self.agents.invalidate().await;
            r
        })
    }

    fn stop<'a>(&'a self, short_id: &'a str) -> BoxFut<'a, Result<(), HostError>> {
        Box::pin(async move {
            let r = cli::stop(short_id).await;
            self.agents.invalidate().await;
            Ok(r?)
        })
    }
}

/// `~/.claude/projects` (`nodal_domain::ports::SessionFiles`).
pub struct HostSessionFiles;

impl SessionFiles for HostSessionFiles {
    fn read_session(&self, session_id: &str, cwd: &str) -> SessionReadout {
        fs::readout::read_session(session_id, cwd)
    }

    fn session_tokens(&self, session_id: &str, cwd: &str) -> Option<i64> {
        fs::readout::session_tokens(session_id, cwd)
    }

    fn session_close(&self, session_id: &str, cwd: &str) -> (SessionReadout, Option<i64>) {
        fs::readout::read_session_close(session_id, cwd)
    }

    fn run_detail(&self, session_id: &str, cwd: &str) -> Result<Option<RunDetail>, HostError> {
        let projects = fs::readout::projects_dir()?;
        Ok(fs::paths::find_session_dir(&projects, cwd, session_id).and_then(|dir| fs::workflow_detail::read_run_detail(&dir)))
    }

    fn launch_blocker(&self, session_id: &str, cwd: &str) -> Result<Option<LaunchBlocker>, HostError> {
        let projects = fs::readout::projects_dir()?;
        Ok(fs::paths::find_session_jsonl(&projects, cwd, session_id)
            .and_then(|p| fs::review_denial::read_workflow_review_denial(&p))
            .map(|workflow| LaunchBlocker::WorkflowReview { workflow }))
    }

    fn agent_transcript(
        &self,
        session_id: &str,
        cwd: &str,
        run_id: &str,
        agent_id: &str,
        limit: usize,
    ) -> Result<Option<Transcript>, HostError> {
        let projects = fs::readout::projects_dir()?;
        let Some(dir) = fs::paths::find_session_dir(&projects, cwd, session_id) else { return Ok(None) };
        Ok(fs::transcript::read_agent_transcript(&dir, run_id, agent_id, limit)?)
    }

    fn find_session_jsonl(&self, projects: &Path, cwd: &str, session_id: &str) -> Option<PathBuf> {
        fs::paths::find_session_jsonl(projects, cwd, session_id)
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

    fn count_new_tool_calls(&self, path: &Path, from: u64) -> io::Result<(u32, u64)> {
        fs::transcript::count_new_tool_calls(path, from)
    }

    fn external_sessions(
        &self,
        repos: &[(String, PathBuf)],
        agents: &[AgentSession],
        app: &AppRuns,
        now: i64,
    ) -> Result<ExternalSessions, HostError> {
        let projects = fs::paths::claude_config_dir()
            .ok_or("Couldn't locate the Claude Code folder ($HOME is not set).")?
            .join("projects");
        Ok(activity::external_sessions_of(repos, agents, &projects, app, now, activity::existing_roots))
    }
}

/// Claude Code config on disk (`nodal_domain::ports::ClaudeConfig`).
pub struct HostClaudeConfig;

impl ClaudeConfig for HostClaudeConfig {
    fn catalog(&self, claude_dir: Option<&Path>, repo: Option<&Path>) -> Vec<ExecutorInfo> {
        catalog::catalog(claude_dir, repo)
    }

    fn find_workflow(&self, claude_dir: Option<&Path>, repo: Option<&Path>, name: &str) -> Option<ExecutorInfo> {
        catalog::find_workflow(claude_dir, repo, name)
    }

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

    fn current_branch(&self, dir: &Path) -> Option<String> {
        git::diff::current_branch(dir)
    }

    fn merge(&self, repo: &Path, wt: &WorktreeRef, message: &str, squash: bool) -> Result<MergeOutcome, HostError> {
        Ok(git::merge::merge(repo, wt, message, squash)?)
    }

    fn push_base(&self, repo: &Path, base: &str) -> Result<String, HostError> {
        Ok(git::merge::push_base(repo, base)?)
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
        path.canonicalize()
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
