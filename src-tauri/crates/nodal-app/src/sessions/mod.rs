//! Sessions context: read-only access to Claude Code sessions and runs' transcripts.
//!
//! `SessionReader` serves the four commands that work without a database (`claude agents`
//! plus the session files under `~/.claude/projects`); it's always registered and holds
//! nothing but the two ports it needs. `Sessions` needs the database too (`get_run_transcript`,
//! `external_sessions`), so it holds the shared `Core`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use nodal_domain::model::activity::{AppRuns, ExternalSessions};
use nodal_domain::model::claude::{LaunchBlocker, RunDetail, RunProgress, RunSummary, Transcript};
use nodal_domain::model::Executor;
use nodal_domain::ports::{ClaudeCli, SessionFiles};
use nodal_domain::sessions::transcript::{
    is_valid_path_id, is_valid_session_id, TRANSCRIPT_DEFAULT_LIMIT, TRANSCRIPT_MAX_LIMIT,
};
use nodal_domain::util::check_id;
use nodal_store::{board, execution::runs, with_db};

use crate::core::{blocking, AppError, Core};

/// Serves `list_runs`, `get_run_detail`, `get_agent_transcript` and `get_launch_blocker`
/// without a database, exactly like today; always registered.
pub struct SessionReader {
    claude: Arc<dyn ClaudeCli>,
    files: Arc<dyn SessionFiles>,
}

impl SessionReader {
    pub fn new(claude: Arc<dyn ClaudeCli>, files: Arc<dyn SessionFiles>) -> Arc<Self> {
        Arc::new(Self { claude, files })
    }

    /// Background sessions (`claude agents --json --all`), most recent first.
    pub async fn list_runs(&self) -> Result<Vec<RunSummary>, AppError> {
        Ok(self.claude.list_sessions().await?)
    }

    /// Detail of the session's most recent workflow. `None` if the session has no folder on
    /// disk yet or didn't launch any workflow.
    pub async fn get_run_detail(
        &self,
        session_id: String,
        cwd: String,
    ) -> Result<Option<RunDetail>, AppError> {
        if !is_valid_session_id(&session_id) {
            return Err(AppError::Message(format!(
                "Invalid session id: {session_id}"
            )));
        }
        let files = self.files.clone();
        tokio::task::spawn_blocking(move || files.run_detail(&session_id, &cwd))
            .await
            .map_err(|e| AppError::Message(format!("Internal error reading the session: {e}")))?
            .map_err(AppError::from)
    }

    /// Says whether the session ended without running its workflow because Claude Code asked
    /// to approve it ("Review dynamic workflow before running"). `None` if there's no
    /// transcript or that rejection doesn't show up. Reads at most the first 4 MB of the main
    /// transcript.
    pub async fn get_launch_blocker(
        &self,
        session_id: String,
        cwd: String,
    ) -> Result<Option<LaunchBlocker>, AppError> {
        if !is_valid_session_id(&session_id) {
            return Err(AppError::Message(format!(
                "Invalid session id: {session_id}"
            )));
        }
        let files = self.files.clone();
        tokio::task::spawn_blocking(move || files.launch_blocker(&session_id, &cwd))
            .await
            .map_err(|e| AppError::Message(format!("Internal error reading the session: {e}")))?
            .map_err(AppError::from)
    }

    /// Transcript of a workflow subagent: prompt, conversation (clipped) and output. `limit`:
    /// max number of items to return, the most recent ones (default 200, max 2000). `None` if
    /// the agent has no file yet.
    pub async fn get_agent_transcript(
        &self,
        session_id: String,
        cwd: String,
        run_id: String,
        agent_id: String,
        limit: Option<u32>,
    ) -> Result<Option<Transcript>, AppError> {
        if !is_valid_session_id(&session_id) {
            return Err(AppError::Message(format!(
                "Invalid session id: {session_id}"
            )));
        }
        if !run_id.starts_with("wf_") || !is_valid_path_id(&run_id) {
            return Err(AppError::Message(format!(
                "Invalid workflow run id: {run_id}"
            )));
        }
        if !is_valid_path_id(&agent_id) {
            return Err(AppError::Message(format!("Invalid agent id: {agent_id}")));
        }
        let limit = limit
            .unwrap_or(TRANSCRIPT_DEFAULT_LIMIT)
            .clamp(1, TRANSCRIPT_MAX_LIMIT) as usize;
        let files = self.files.clone();
        tokio::task::spawn_blocking(move || {
            files.agent_transcript(&session_id, &cwd, &run_id, &agent_id, limit)
        })
        .await
        .map_err(|e| AppError::Message(format!("Internal error reading the transcript: {e}")))?
        .map_err(AppError::from)
    }
}

/// Byte offset and cumulative tool-call count `run_progress` last read a run's session at
/// (P03): the next poll only scans what was appended since.
#[derive(Default, Clone, Copy)]
struct ProgressCursor {
    offset: u64,
    tool_calls: u32,
}

/// Needs the database: `get_run_transcript` and `external_sessions`.
pub struct Sessions {
    core: Arc<Core>,
    /// One cursor per run, alive as long as the app runs. Bounded in practice: it only grows
    /// for runs `run_progress` was actually polled for (live, non-workflow runs), the same
    /// small set `RunMonitor` shows at once.
    progress: Mutex<HashMap<String, ProgressCursor>>,
}

impl Sessions {
    pub(crate) fn new(core: Arc<Core>) -> Self {
        Self {
            core,
            progress: Mutex::new(HashMap::new()),
        }
    }

    /// Transcript of an agent, Claude or reviewer run (the main session). Workflows have one
    /// per agent: `get_agent_transcript`. `None` if the session has no file yet. `limit`: most
    /// recent items (default 200, max 2000).
    pub async fn get_run_transcript(
        &self,
        run_id: String,
        limit: Option<u32>,
    ) -> Result<Option<Transcript>, AppError> {
        check_id(&run_id, "run")?;
        let run = with_db(&self.core.db, move |c| runs::get(c, &run_id)).await?;
        let label = match &run.executor {
            Executor::Workflow { .. } => {
                return Err(
                    "Workflow runs have one transcript per agent: open it from the run detail."
                        .into(),
                )
            }
            Executor::Agent { name, .. } => name.clone(),
            Executor::Claude => "Claude".into(),
        };
        let Some(sid) = run.session_id.clone().filter(|s| is_valid_session_id(s)) else {
            return Ok(None);
        };
        let Some(claude_dir) = self.core.env.claude_dir.clone() else {
            return Err("Couldn't locate the Claude Code folder ($HOME is not set).".into());
        };
        let limit = limit
            .unwrap_or(TRANSCRIPT_DEFAULT_LIMIT)
            .clamp(1, TRANSCRIPT_MAX_LIMIT) as usize;
        let sessions = self.core.sessions.clone();
        blocking(move || {
            let Some(path) =
                sessions.find_session_jsonl(&claude_dir.join("projects"), &run.cwd, &sid)
            else {
                return Ok(None);
            };
            sessions
                .read_session_transcript(
                    &path,
                    &run.id,
                    Some(label),
                    run.options.model.clone(),
                    limit,
                )
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(AppError::from)
    }

    /// Tool calls of a live run so far, read with an incremental cursor (byte offset per
    /// run) instead of re-parsing the whole transcript on every poll (P03). `None` if the
    /// run has no session yet. Errors like `get_run_transcript` (invalid id, workflow run,
    /// missing Claude Code folder).
    pub async fn run_progress(&self, run_id: String) -> Result<Option<RunProgress>, AppError> {
        check_id(&run_id, "run")?;
        let run = with_db(&self.core.db, move |c| runs::get(c, &run_id)).await?;
        if matches!(run.executor, Executor::Workflow { .. }) {
            return Err(
                "Workflow runs report progress per phase: open the run detail instead.".into(),
            );
        }
        let Some(sid) = run.session_id.clone().filter(|s| is_valid_session_id(s)) else {
            return Ok(None);
        };
        let Some(claude_dir) = self.core.env.claude_dir.clone() else {
            return Err("Couldn't locate the Claude Code folder ($HOME is not set).".into());
        };
        let from = self.cursor_offset(&run.id);
        let sessions = self.core.sessions.clone();
        let cwd = run.cwd.clone();
        let (delta, offset) = blocking(move || {
            let Some(path) = sessions.find_session_jsonl(&claude_dir.join("projects"), &cwd, &sid) else {
                return Ok((0, from));
            };
            sessions
                .count_new_tool_calls(&path, from)
                .map_err(|e| format!("Couldn't read the transcript: {e}"))
        })
        .await
        .map_err(AppError::from)?;
        Ok(Some(self.advance_cursor(run.id, delta, offset)))
    }

    fn cursor_offset(&self, run_id: &str) -> u64 {
        self.progress
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(run_id)
            .map(|c| c.offset)
            .unwrap_or(0)
    }

    fn advance_cursor(&self, run_id: String, delta: u32, offset: u64) -> RunProgress {
        let mut cache = self.progress.lock().unwrap_or_else(|p| p.into_inner());
        let cursor = cache.entry(run_id).or_default();
        cursor.tool_calls += delta;
        cursor.offset = offset;
        RunProgress {
            tool_calls: cursor.tool_calls,
        }
    }

    /// Runs launched by the app (`runs` table). If the database read fails, nothing is
    /// marked (kept from today's `activity::app_run_refs`).
    async fn app_run_refs(&self) -> AppRuns {
        let mut refs = AppRuns::default();
        match with_db(&self.core.db, |c| runs::launched_refs(c)).await {
            Ok(list) => {
                for (r, s) in list {
                    refs.add(r, s);
                }
            }
            Err(e) => eprintln!("activity: {e}"),
        }
        refs
    }

    /// Claude Code sessions started outside the app in the project's repos (`None`: every
    /// project's), with a single `claude agents`.
    pub async fn external_sessions(
        &self,
        project_id: Option<String>,
    ) -> Result<ExternalSessions, AppError> {
        if let Some(id) = &project_id {
            check_id(id, "project")?;
        }
        let repos: Vec<(String, PathBuf)> = with_db(&self.core.db, move |c| {
            if let Some(pid) = &project_id {
                board::projects::get(c, pid)?;
            }
            // Every project: skip archived ones (their runs don't show on Runs either).
            let live: Option<HashSet<String>> = match project_id {
                Some(_) => None,
                None => Some(
                    board::projects::list(c, false)?
                        .into_iter()
                        .map(|p| p.id)
                        .collect(),
                ),
            };
            Ok(board::repos::list(c, project_id.as_deref())?
                .into_iter()
                .filter(|r| live.as_ref().is_none_or(|l| l.contains(&r.project_id)))
                .map(|r| (r.id, PathBuf::from(r.path)))
                .collect())
        })
        .await?;
        if repos.is_empty() {
            return Ok(ExternalSessions {
                repos: vec![],
                generated_at: self.core.clock.now_ms(),
            });
        }
        let agents = self.core.claude.list_agent_sessions().await?;
        let refs = self.app_run_refs().await;
        let sessions = self.core.sessions.clone();
        let now = self.core.clock.now_ms();
        tokio::task::spawn_blocking(move || sessions.external_sessions(&repos, &agents, &refs, now))
            .await
            .map_err(|e| AppError::Message(format!("Internal error reading activity: {e}")))?
            .map_err(AppError::from)
    }
}

#[cfg(test)]
mod tests;
