//! Claude Code background runs: launch them (`claude --bg`), list them
//! (`claude agents`) and follow the workflow they run by reading their session files.

pub mod claude_bin;
pub(crate) mod claude_fs;
pub mod claude_trust;
pub mod options;
pub mod terminal;
pub mod types;
pub mod workflows;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tauri::State;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::mpsc;

use crate::db::{self, Db};
use types::{LaunchBlocker, LaunchOptions, RunDetail, RunRef, RunSummary, Transcript};

const LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);
const LIST_TIMEOUT: Duration = Duration::from_secs(15);

/// Forwards each line of a child stream to the channel.
fn forward_lines<R: AsyncRead + Unpin + Send + 'static>(stream: R, tx: mpsc::UnboundedSender<String>) {
    tauri::async_runtime::spawn(async move {
        let mut lines = BufReader::new(stream).lines();
        // Drain until EOF even if nobody listens anymore: closing the pipe would give EPIPE to
        // `claude` (or to the background session, if it inherited the pipe).
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = tx.send(line);
        }
    });
}

/// Executor flags (`--agent`, `--disallowedTools`, …) passed on top of the options.
/// Each value separate; the ones built here are already validated.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtraFlags {
    /// `--agent <name>`.
    pub agent: Option<String>,
    /// `--allowedTools A B C`: each rule as its own argument, as the permissions docs say
    /// (`Bash(npm test:*)` contains a space); the `--` before the prompt ends the list.
    pub allowed_tools: Vec<String>,
    /// `--disallowedTools A,B,C` (comma-separated: the option is variadic and, with
    /// spaces, swallows the prompt; verified in the spike with 2.1.281).
    pub disallowed_tools: Vec<String>,
    /// `--append-system-prompt <text>`: a single argument, unescaped (there's no shell);
    /// verified with 2.1.282, also with `--agent` (`real_launch_with_append_system_prompt`).
    pub append_system_prompt: Option<String>,
}

impl ExtraFlags {
    pub fn to_args(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(a) = &self.agent {
            out.push("--agent".into());
            out.push(a.clone());
        }
        if !self.allowed_tools.is_empty() {
            out.push("--allowedTools".into());
            out.extend(self.allowed_tools.iter().cloned());
        }
        if !self.disallowed_tools.is_empty() {
            out.push("--disallowedTools".into());
            out.push(self.disallowed_tools.join(","));
        }
        if let Some(p) = &self.append_system_prompt {
            out.push("--append-system-prompt".into());
            out.push(p.clone());
        }
        out
    }
}

/// Why a `claude --bg` failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchError {
    pub message: String,
    /// It didn't return the id in time: the session may have started anyway.
    pub timed_out: bool,
}

impl From<String> for LaunchError {
    fn from(message: String) -> Self {
        LaunchError { message, timed_out: false }
    }
}

impl From<&str> for LaunchError {
    fn from(message: &str) -> Self {
        message.to_string().into()
    }
}

/// Like `launch_bg`, with the error as text.
pub async fn launch_with(cwd: String, prompt: String, opts: &LaunchOptions, extra: &ExtraFlags) -> Result<RunRef, String> {
    launch_bg(cwd, prompt, opts, extra).await.map_err(|e| e.message)
}

/// Launches `claude --bg [flags] -- <prompt>` in `cwd` and returns the session's short id.
/// Everything goes as separate arguments (no shell), so there's nothing to escape; the
/// flags are validated against the allowed lists in `options`. The `--` ends the variadic
/// options before the prompt.
pub async fn launch_bg(cwd: String, prompt: String, opts: &LaunchOptions, extra: &ExtraFlags) -> Result<RunRef, LaunchError> {
    let dir = Path::new(&cwd);
    if !dir.is_absolute() {
        return Err(format!("The folder must be an absolute path: {cwd}").into());
    }
    if !dir.is_dir() {
        return Err(format!("The folder doesn't exist or isn't a directory: {cwd}").into());
    }
    if prompt.trim().is_empty() {
        return Err("The prompt is empty.".into());
    }
    // `claude` would read it as an option, not as the prompt.
    if prompt.trim_start().starts_with('-') {
        return Err("The prompt can't start with \"-\".".into());
    }
    let flags = options::to_args(opts)?;

    let mut cmd = claude_bin::claude_command()?;
    cmd.arg("--bg")
        .args(&flags)
        .args(extra.to_args())
        .arg("--")
        .arg(&prompt)
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("Couldn't run `claude --bg`: {e}"))?;

    // Don't wait for EOF: seeing the `backgrounded · <id>` line on either stream is enough
    // (there's no guarantee the background session releases the pipes).
    let (tx, mut rx) = mpsc::unbounded_channel();
    if let Some(out) = child.stdout.take() {
        forward_lines(out, tx.clone());
    }
    if let Some(err) = child.stderr.take() {
        forward_lines(err, tx);
    }
    let found = tokio::time::timeout(LAUNCH_TIMEOUT, async {
        let mut seen = String::new();
        while let Some(line) = rx.recv().await {
            if let Some(id) = claude_fs::parse_bg_line(&line) {
                return Ok(id);
            }
            seen.push_str(&line);
            seen.push('\n');
        }
        Err(seen)
    })
    .await;

    match found {
        Ok(Ok(id)) => {
            // Reap the process when it exits, without blocking the response.
            tauri::async_runtime::spawn(async move {
                let _ = child.wait().await;
            });
            Ok(RunRef { id, cwd })
        }
        Ok(Err(seen)) => {
            let status = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
            if let Some(id) = claude_fs::parse_bare_id(&seen) {
                return Ok(RunRef { id, cwd });
            }
            let _ = child.start_kill();
            let code = match status {
                Ok(Ok(s)) => s.code().map(|c| format!(" (exit code {c})")).unwrap_or_default(),
                _ => String::new(),
            };
            let seen: String = seen.trim().chars().take(500).collect();
            Err(format!("`claude --bg` exited without returning the session id{code}: {seen}").into())
        }
        Err(_) => {
            let _ = child.start_kill();
            Err(LaunchError {
                message: format!(
                    "`claude --bg` didn't return the session id within {} s. It may have launched anyway: check the runs list before retrying.",
                    LAUNCH_TIMEOUT.as_secs()
                ),
                timed_out: true,
            })
        }
    }
}

/// Options of the repo whose root is `cwd` (also comparing the canonicalized path).
fn repo_options_for(conn: &rusqlite::Connection, cwd: &str) -> Result<LaunchOptions, db::DbError> {
    let mut keys = vec![cwd.trim_end_matches('/').to_string()];
    if let Ok(c) = Path::new(cwd).canonicalize() {
        keys.push(c.to_string_lossy().into_owned());
    }
    for k in keys {
        if let Some(r) = crate::db::queries::repos::find_by_path(conn, &k)? {
            return Ok(r.launch);
        }
    }
    Ok(LaunchOptions::default())
}

/// Manual launch (outside the queue): uses the repo's model/effort/permission mode if
/// `cwd` is a registered repo.
#[tauri::command]
pub async fn launch_run(db: State<'_, Db>, cwd: String, prompt: String) -> Result<RunRef, String> {
    let key = cwd.clone();
    // An unreadable database doesn't block a manual launch: it launches without flags.
    let opts = match db::with_db(&db, move |c| repo_options_for(c, &key)).await {
        Ok(o) => o,
        Err(e) => {
            eprintln!("launch_run: {e}; launching without repo options");
            LaunchOptions::default()
        }
    };
    launch_with(cwd, prompt, &opts, &ExtraFlags::default()).await
}

/// Background sessions (`claude agents --json --all`), most recent first.
#[tauri::command]
pub async fn list_runs() -> Result<Vec<RunSummary>, String> {
    let mut cmd = claude_bin::claude_command()?;
    cmd.args(["agents", "--json", "--all"]);
    let out = claude_bin::output_with_timeout(cmd, LIST_TIMEOUT, "`claude agents`").await?;
    if !out.status.success() {
        return Err(format!("`claude agents` failed: {}", claude_bin::error_text(&out)));
    }
    claude_fs::parse_agents_json(&String::from_utf8_lossy(&out.stdout))
}

fn projects_dir() -> Result<PathBuf, String> {
    Ok(claude_fs::claude_config_dir()
        .ok_or("Couldn't locate the Claude Code folder ($HOME is not set).")?
        .join("projects"))
}

/// What a finished session left behind, read from disk (blocking). For the queue.
#[derive(Debug, Default, Clone)]
pub struct SessionReadout {
    /// Detail of the most recent workflow (only if the session ran one).
    pub detail: Option<RunDetail>,
    /// Last assistant message in the main transcript.
    pub last_message: Option<String>,
    /// Workflow name if Claude Code asked to approve it and it never ran.
    pub blocker: Option<Option<String>>,
}

pub fn read_session(session_id: &str, cwd: &str) -> SessionReadout {
    if !claude_fs::is_valid_session_id(session_id) {
        return SessionReadout::default();
    }
    let Ok(projects) = projects_dir() else { return SessionReadout::default() };
    let detail = claude_fs::find_session_dir(&projects, cwd, session_id).and_then(|d| claude_fs::read_run_detail(&d));
    let jsonl = claude_fs::find_session_jsonl(&projects, cwd, session_id);
    SessionReadout {
        detail,
        last_message: jsonl.as_deref().and_then(claude_fs::read_last_assistant_text),
        blocker: jsonl.as_deref().and_then(claude_fs::read_workflow_review_denial),
    }
}

/// Tokens of the session's main transcript (blocking). `None` if there's no file or usage.
pub fn session_tokens(session_id: &str, cwd: &str) -> Option<i64> {
    if !claude_fs::is_valid_session_id(session_id) {
        return None;
    }
    let projects = projects_dir().ok()?;
    claude_fs::read_usage_tokens(&claude_fs::find_session_jsonl(&projects, cwd, session_id)?)
}

/// Detail of the session's most recent workflow. `None` if the session has no folder on
/// disk yet or didn't launch any workflow.
#[tauri::command]
pub async fn get_run_detail(session_id: String, cwd: String) -> Result<Option<RunDetail>, String> {
    if !claude_fs::is_valid_session_id(&session_id) {
        return Err(format!("Invalid session id: {session_id}"));
    }
    tauri::async_runtime::spawn_blocking(move || -> Result<Option<RunDetail>, String> {
        let projects = projects_dir()?;
        Ok(claude_fs::find_session_dir(&projects, &cwd, &session_id).and_then(|dir| claude_fs::read_run_detail(&dir)))
    })
    .await
    .map_err(|e| format!("Internal error reading the session: {e}"))?
}

/// Says whether the session ended without running its workflow because Claude Code asked to
/// approve it ("Review dynamic workflow before running"). `None` if there's no transcript or
/// that rejection doesn't show up. Reads at most the first 4 MB of the main transcript.
#[tauri::command]
pub async fn get_launch_blocker(session_id: String, cwd: String) -> Result<Option<LaunchBlocker>, String> {
    if !claude_fs::is_valid_session_id(&session_id) {
        return Err(format!("Invalid session id: {session_id}"));
    }
    tauri::async_runtime::spawn_blocking(move || -> Result<Option<LaunchBlocker>, String> {
        let projects = projects_dir()?;
        Ok(claude_fs::find_session_jsonl(&projects, &cwd, &session_id)
            .and_then(|p| claude_fs::read_workflow_review_denial(&p))
            .map(|workflow| LaunchBlocker::WorkflowReview { workflow }))
    })
    .await
    .map_err(|e| format!("Internal error reading the session: {e}"))?
}

/// Transcript of a workflow subagent: prompt, conversation (clipped) and output.
/// `limit`: max number of items to return, the most recent ones (default 200, max 2000).
/// `None` if the agent has no file yet.
#[tauri::command]
pub async fn get_agent_transcript(
    session_id: String,
    cwd: String,
    run_id: String,
    agent_id: String,
    limit: Option<u32>,
) -> Result<Option<Transcript>, String> {
    if !claude_fs::is_valid_session_id(&session_id) {
        return Err(format!("Invalid session id: {session_id}"));
    }
    if !run_id.starts_with("wf_") || !claude_fs::is_valid_path_id(&run_id) {
        return Err(format!("Invalid workflow run id: {run_id}"));
    }
    if !claude_fs::is_valid_path_id(&agent_id) {
        return Err(format!("Invalid agent id: {agent_id}"));
    }
    let limit = limit
        .unwrap_or(claude_fs::TRANSCRIPT_DEFAULT_LIMIT)
        .clamp(1, claude_fs::TRANSCRIPT_MAX_LIMIT) as usize;
    tauri::async_runtime::spawn_blocking(move || -> Result<Option<Transcript>, String> {
        let projects = projects_dir()?;
        let Some(dir) = claude_fs::find_session_dir(&projects, &cwd, &session_id) else {
            return Ok(None);
        };
        claude_fs::read_agent_transcript(&dir, &run_id, &agent_id, limit)
    })
    .await
    .map_err(|e| format!("Internal error reading the transcript: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_system_prompt_is_one_arg_after_the_variadic_lists() {
        let extra = ExtraFlags {
            allowed_tools: vec!["Bash(npm test:*)".into()],
            append_system_prompt: Some("Don't ask: stop.".into()),
            ..Default::default()
        };
        assert_eq!(extra.to_args(), ["--allowedTools", "Bash(npm test:*)", "--append-system-prompt", "Don't ask: stop."]);
        assert!(ExtraFlags::default().to_args().is_empty());
    }

    /// Against the real `claude` and this machine's data: `cargo test -- --ignored`.
    /// Read-only (`claude agents` + files); doesn't launch runs.
    #[test]
    #[ignore]
    fn real_list_and_detail() {
        let runs = tauri::async_runtime::block_on(list_runs()).expect("list_runs");
        eprintln!("{} background runs", runs.len());
        for r in runs.iter().take(5) {
            let cwd = r.cwd.clone().unwrap_or_default();
            let d = tauri::async_runtime::block_on(get_run_detail(r.session_id.clone(), cwd)).expect("detail");
            eprintln!(
                "{} {:?} {:?} -> {:?}",
                r.id,
                r.state,
                r.name,
                d.map(|d| (d.workflow_id, d.source, d.current_phase, d.agents.len()))
            );
        }
        let err = tauri::async_runtime::block_on(launch_with("/no/such/dir".into(), "x".into(), &LaunchOptions::default(), &ExtraFlags::default()))
            .unwrap_err();
        assert_eq!(err, "The folder doesn't exist or isn't a directory: /no/such/dir");
    }

    /// Against the real `claude`: `cargo test -- --ignored`. Launches two runs (plain and with
    /// `--agent code-reviewer`, which must exist in `~/.claude/agents`) and stops them.
    #[test]
    #[ignore]
    fn real_launch_with_append_system_prompt() {
        let cwd = env!("CARGO_MANIFEST_DIR").to_string();
        let prompt = "Reply with the single word OK.".to_string();
        let sp = Some(crate::work::launch::UNATTENDED_SYSTEM_PROMPT.to_string());
        for agent in [None, Some("code-reviewer".to_string())] {
            let extra = ExtraFlags { agent: agent.clone(), append_system_prompt: sp.clone(), ..Default::default() };
            let run = tauri::async_runtime::block_on(launch_with(cwd.clone(), prompt.clone(), &LaunchOptions::default(), &extra))
                .unwrap_or_else(|e| panic!("launch with agent {agent:?}: {e}"));
            eprintln!("agent {agent:?} -> backgrounded · {}", run.id);
            tauri::async_runtime::block_on(terminal::stop(&run.id)).expect("stop");
        }
    }

    #[test]
    fn transcript_rejects_traversal_ids() {
        let call = |run: &str, agent: &str| {
            tauri::async_runtime::block_on(get_agent_transcript(
                "ddb91222-57b2-4ae4-a0bb-d1c5993dc1c8".into(),
                "/x".into(),
                run.into(),
                agent.into(),
                None,
            ))
        };
        assert!(call("wf_../../etc", "a1").unwrap_err().contains("Invalid workflow run id"));
        assert!(call("../wf_x", "a1").unwrap_err().contains("Invalid workflow run id"));
        assert!(call("wf_abc", "../../x").unwrap_err().contains("Invalid agent id"));
        assert!(call("wf_abc", "a/b").unwrap_err().contains("Invalid agent id"));
    }
}
