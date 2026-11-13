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
use crate::model::chat::{ChatEnvelope, ChatLive, ChatSpec, ClaudeDefaults};
use crate::model::claude::Transcript;
use crate::model::events::ChangeKind;
use crate::model::executors::AgentDef;
use crate::model::providers::{ExternalItem, ImportQuery, Page, ProviderResult};
use crate::model::{DiffStat, DirEntry, RepoFile, RepoWorktree, ExternalState, ScopeRef, WorktreeRef};

pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// `util::now_ms`.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

/// `~/.claude/projects`. Blocking.
pub trait SessionFiles: Send + Sync {
    fn find_session_jsonl(&self, projects: &Path, cwd: &str, session_id: &str) -> Option<PathBuf>;
    /// `(cwd, gitBranch)` of the most recent session entries that carry them (from the file's
    /// tail). Default: unknown.
    fn session_location(&self, path: &Path) -> (Option<String>, Option<String>) {
        let _ = path;
        (None, None)
    }
    fn read_session_transcript(
        &self,
        path: &Path,
        id: &str,
        label: Option<String>,
        model: Option<String>,
        limit: usize,
    ) -> Result<Option<Transcript>, HostError>;
    fn session_title(&self, path: &Path) -> Option<String>;
}

/// Claude Code config on disk. Blocking.
pub trait ClaudeConfig: Send + Sync {
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
    /// HEAD's commits not in `base`, or the latest ones without a base (newest first).
    fn log(&self, dir: &Path, base: Option<&str>) -> Result<Vec<CommitInfo>, HostError>;
    /// A commit's patch (unified diff text), `sha` hex only.
    fn show_commit(&self, dir: &Path, sha: &str) -> Result<String, HostError>;
    fn current_branch(&self, dir: &Path) -> Option<String>;
    /// Direct children of `rel` (relative to `repo`, "" = root): dirs first, then files, each
    /// sorted case-insensitively; `.git` and git-ignored entries left out; capped at
    /// `DIR_ENTRIES_MAX`. Errors on absolute paths, `..`, or anything resolving outside `repo`.
    fn list_dir(&self, repo: &Path, rel: &str) -> Result<Vec<DirEntry>, HostError>;
    /// Every file of the checkout (tracked plus untracked, git-ignored left out), relative,
    /// capped at `REPO_FILES_MAX`; for the quick-open search.
    fn list_files(&self, repo: &Path) -> Result<Vec<String>, HostError>;
    /// A regular file under `repo` (same path rules as `list_dir`), first 1 MB as UTF-8 text;
    /// `binary` files come back empty. Git-ignored files are readable.
    fn read_file(&self, repo: &Path, rel: &str) -> Result<RepoFile, HostError>;
    /// Atomically overwrites an existing text file (same path rules). Errors starting with
    /// "File changed on disk" when `version` is stale and not `force`.
    fn write_file(&self, repo: &Path, rel: &str, content: &str, version: &str, force: bool) -> Result<RepoFile, HostError>;
    /// The repo's worktrees, main first; prunable or missing ones left out.
    fn list_worktrees(&self, repo: &Path) -> Result<Vec<RepoWorktree>, HostError>;
    /// Discards one file's uncommitted changes (see `git::ls::revert_file`).
    fn revert_file(&self, root: &Path, rel: &str) -> Result<(), HostError>;
    /// Whether `dir` is inside a linked worktree (its `.git` is a file).
    fn is_worktree(&self, dir: &Path) -> bool;
    /// Uncommitted changes in `dir` vs HEAD, untracked non-ignored files included. Cheap (no
    /// full diff); zeros when `dir` isn't a git repo.
    fn changes(&self, dir: &Path) -> DiffStat;
    fn merge(
        &self,
        repo: &Path,
        wt: &WorktreeRef,
        message: &str,
        squash: bool,
    ) -> Result<MergeOutcome, HostError>;
    fn push_base(&self, repo: &Path, base: &str) -> Result<String, HostError>;
    /// Stages everything (`git add -A`) and commits it; returns the new commit. Errors with
    /// "Nothing to commit." or git's own message (hooks, identity).
    /// Commits everything, or only `paths` (relative) when given.
    fn commit_all(&self, dir: &Path, message: &str, paths: &[String]) -> Result<CommitInfo, HostError>;
    /// The remote's default branch when known locally (`origin/HEAD`).
    fn default_branch(&self, dir: &Path) -> Option<String>;
    /// Pushes `branch` to `origin` and opens a PR against `base` with the GitHub CLI (or finds
    /// the existing one). Returns the PR URL.
    fn open_pr(&self, dir: &Path, base: &str, branch: &str, title: &str, body: &str) -> Result<String, HostError>;
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
