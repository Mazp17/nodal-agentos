//! Ports: the only way nodal-app reaches external I/O. Object-safe, used as `Arc<dyn …>`.
//! Blocking methods are called exactly where the old code did the I/O (inside
//! `spawn_blocking`/`with_db` closures); async ones return `BoxFut`.
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use crate::diff::CommitInfo;
use crate::error::HostError;
use crate::execution::worktree::{MergeOutcome, WorktreeStatus};
use crate::model::activity::{AgentSession, AppRuns, ExternalSessions};
use crate::model::chat::{ChatEnvelope, ChatLive, ChatSpec, ClaudeDefaults};
use crate::model::claude::{
    ExtraFlags, LaunchBlocker, LaunchError, RunDetail, RunRef, RunSummary, SessionReadout,
    Transcript,
};
use crate::model::events::ChangeKind;
use crate::model::executors::{AgentDef, ExecutorInfo};
use crate::model::providers::{ExternalItem, ImportQuery, Page, ProviderResult};
use crate::model::{ExternalState, LaunchOptions, ScopeRef, WorktreeRef};

pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// `util::now_ms`.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

/// The `claude` CLI. Bodies are today's functions verbatim.
pub trait ClaudeCli: Send + Sync {
    /// `runs::list_runs` (claude agents --json --all → claude_fs::parse_agents_json).
    fn list_sessions(&self) -> BoxFut<'_, Result<Vec<RunSummary>, HostError>>;
    /// `activity::list_agents` (same command → claude_sessions::parse_agents). Not unified on purpose.
    fn list_agent_sessions(&self) -> BoxFut<'_, Result<Vec<AgentSession>, HostError>>;
    /// `runs::launch_bg`.
    fn launch_bg<'a>(
        &'a self,
        cwd: String,
        prompt: String,
        opts: &'a LaunchOptions,
        extra: &'a ExtraFlags,
    ) -> BoxFut<'a, Result<RunRef, LaunchError>>;
    /// `runs::terminal::stop` (validates the id).
    fn stop<'a>(&'a self, short_id: &'a str) -> BoxFut<'a, Result<(), HostError>>;
}

/// `~/.claude/projects`. Blocking.
pub trait SessionFiles: Send + Sync {
    fn read_session(&self, session_id: &str, cwd: &str) -> SessionReadout; // runs::read_session
    fn session_tokens(&self, session_id: &str, cwd: &str) -> Option<i64>; // runs::session_tokens
    fn run_detail(&self, session_id: &str, cwd: &str) -> Result<Option<RunDetail>, HostError>; // get_run_detail closure body
    fn launch_blocker(
        &self,
        session_id: &str,
        cwd: &str,
    ) -> Result<Option<LaunchBlocker>, HostError>; // get_launch_blocker closure body
    fn agent_transcript(
        &self,
        session_id: &str,
        cwd: &str,
        run_id: &str,
        agent_id: &str,
        limit: usize,
    ) -> Result<Option<Transcript>, HostError>; // get_agent_transcript closure body
    fn find_session_jsonl(&self, projects: &Path, cwd: &str, session_id: &str) -> Option<PathBuf>;
    fn read_session_transcript(
        &self,
        path: &Path,
        id: &str,
        label: Option<String>,
        model: Option<String>,
        limit: usize,
    ) -> Result<Option<Transcript>, HostError>;
    fn session_title(&self, path: &Path) -> Option<String>;
    /// external_sessions closure body: projects dir lookup + external_sessions_of(.., existing_roots).
    fn external_sessions(
        &self,
        repos: &[(String, PathBuf)],
        agents: &[AgentSession],
        app: &AppRuns,
        now: i64,
    ) -> Result<ExternalSessions, HostError>;
}

/// Claude Code config on disk. Blocking.
pub trait ClaudeConfig: Send + Sync {
    fn catalog(&self, claude_dir: Option<&Path>, repo: Option<&Path>) -> Vec<ExecutorInfo>; // executors::catalog
    fn find_workflow(
        &self,
        claude_dir: Option<&Path>,
        repo: Option<&Path>,
        name: &str,
    ) -> Option<ExecutorInfo>;
    fn list_agents(&self, claude_dir: Option<&Path>, repo: Option<&Path>) -> Vec<AgentDef>;
    fn claude_defaults(&self, claude_dir: Option<&Path>, repo: Option<&Path>) -> ClaudeDefaults; // claude_settings::read
}

/// git with timeout. Blocking. Errors are today's texts.
pub trait Git: Send + Sync {
    fn require_git_root(&self, path: &str) -> Result<PathBuf, HostError>;
    fn ensure_worktree(
        &self,
        repo: &Path,
        dir: &Path,
        branch: &str,
        existing: Option<&WorktreeRef>,
    ) -> Result<WorktreeRef, HostError>;
    fn cleanup_worktree(&self, repo: &Path, wt: &WorktreeRef) -> Result<(), HostError>;
    fn worktree_status(
        &self,
        repo: &Path,
        wt: Option<&WorktreeRef>,
    ) -> Result<WorktreeStatus, HostError>;
    fn current_base(&self, repo: &Path) -> Result<String, HostError>;
    fn diff(&self, cwd: &Path, base: Option<&str>) -> Result<(String, bool), HostError>; // work::diff::collect
    fn diff_branch(&self, repo: &Path, base: &str, branch: &str) -> Result<String, HostError>; // collect_branch
    fn commits(&self, dir: &Path, base: &str, head: &str) -> Result<Vec<CommitInfo>, HostError>;
    fn current_branch(&self, dir: &Path) -> Option<String>;
    fn merge(
        &self,
        repo: &Path,
        wt: &WorktreeRef,
        message: &str,
        squash: bool,
    ) -> Result<MergeOutcome, HostError>;
    fn push_base(&self, repo: &Path, base: &str) -> Result<String, HostError>;
}

/// Files Nodal owns (plans, stopped patches) + plan files inside repos. Blocking.
pub trait PlanFiles: Send + Sync {
    fn is_file(&self, path: &Path) -> bool;
    fn read_plan(&self, path: &Path) -> Result<String, HostError>; // work::ops::read_plan
    fn validate_plan_file(&self, repo: &Path, path: &str) -> Result<PathBuf, HostError>; // work::validate::plan_file
    fn write_atomic(&self, path: &Path, contents: &[u8]) -> Result<(), HostError>; // util::write_atomic
    fn write_plan(&self, path: &Path, content: &str) -> io::Result<bool>; // providers::plan::write_plan
    fn remove_dir_all(&self, dir: &Path) -> io::Result<()>;
    fn remove_file(&self, path: &Path) -> io::Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
}

/// Read-only probing of user paths. Blocking.
pub trait LocalFs: Send + Sync {
    fn is_dir(&self, path: &Path) -> bool;
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf>;
    fn canonical_dir(&self, path: &str) -> Result<PathBuf, HostError>; // util::paths::canonical_dir
    fn expand_home(&self, path: &str) -> PathBuf; // util::paths::expand_home
    fn test_commands(&self, cwd: &Path) -> Vec<String>; // work::launch::test_commands
    fn nodal_mcp_bin(&self) -> Option<PathBuf>; // chats::context::nodal_mcp_bin
}

/// `nodal://changed` (events::Events API, debounced by the shell adapter).
pub trait ChangeNotifier: Send + Sync {
    fn notify(&self, kind: ChangeKind, project_id: Option<&str>);
    fn notify_all(&self, kinds: &[ChangeKind], project_id: Option<&str>) {
        for k in kinds {
            self.notify(*k, project_id);
        }
    }
}

/// `nodal://chat`.
pub trait ChatSink: Send + Sync {
    fn emit(&self, ev: ChatEnvelope);
}

/// App-side effects of a chat process (bodies of Chats::save_session / refresh_title).
pub trait ChatHooks: Send + Sync {
    fn session_started(&self, chat_id: &str, session_id: String, project_id: String);
    fn turn_ended(&self, chat_id: &str, cwd: PathBuf, project_id: String);
}

/// One `claude -p` stream-json process per active chat (chats::process::Chats API).
pub trait ChatRuntime: Send + Sync {
    fn send(
        &self,
        chat_id: &str,
        project_id: &str,
        session_id: Option<&str>,
        spec: ChatSpec,
        text: &str,
    ) -> Result<(), HostError>;
    fn respond(
        &self,
        chat_id: &str,
        request_id: &str,
        allow: bool,
        message: Option<&str>,
    ) -> Result<(), HostError>;
    fn interrupt(&self, chat_id: &str) -> Result<(), HostError>;
    fn stop(&self, chat_id: &str);
    fn stop_project(&self, project_id: &str);
    fn live(&self, chat_id: &str) -> ChatLive;
    fn start_reaper(&self);
}

/// Keychain backend (secrets::SecretBackend). Blocking.
pub trait SecretStore: Send + Sync + 'static {
    fn read(&self, account: &str) -> Result<Option<String>, HostError>;
    fn write(&self, account: &str, value: &str) -> Result<(), HostError>;
    fn delete(&self, account: &str) -> Result<(), HostError>;
}

/// Object-safe task provider (was `#[allow(async_fn_in_trait)] trait TaskProvider` + enum dispatch).
pub trait TaskProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn status(&self) -> BoxFut<'_, ProviderResult<String>>;
    fn scopes(&self) -> BoxFut<'_, ProviderResult<Vec<ScopeRef>>>;
    fn states<'a>(&'a self, scope: &'a ScopeRef) -> BoxFut<'a, ProviderResult<Vec<ExternalState>>>;
    fn rule_projects<'a>(
        &'a self,
        scope: &'a ScopeRef,
    ) -> BoxFut<'a, ProviderResult<Vec<ScopeRef>>>;
    fn list_importable<'a>(&'a self, query: &'a ImportQuery) -> BoxFut<'a, ProviderResult<Page>>;
    fn pull<'a>(
        &'a self,
        external_ids: &'a [String],
    ) -> BoxFut<'a, ProviderResult<Vec<ExternalItem>>>;
    fn fetch<'a>(
        &'a self,
        external_id: &'a str,
    ) -> BoxFut<'a, ProviderResult<Option<ExternalItem>>> {
        Box::pin(async move {
            let ids = [external_id.to_string()];
            Ok(self.pull(&ids).await?.into_iter().next())
        })
    }
    fn set_state<'a>(
        &'a self,
        external_id: &'a str,
        state_id: &'a str,
    ) -> BoxFut<'a, ProviderResult<ExternalState>>;
    fn comment<'a>(&'a self, external_id: &'a str, body: &'a str)
        -> BoxFut<'a, ProviderResult<()>>;
    fn has_comment_with<'a>(
        &'a self,
        external_id: &'a str,
        marker: &'a str,
    ) -> BoxFut<'a, ProviderResult<bool>>;
}

pub trait ProviderFactory: Send + Sync {
    fn name(&self) -> &'static str;
    fn build(&self, key: String) -> Arc<dyn TaskProvider>;
}
