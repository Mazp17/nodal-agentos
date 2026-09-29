//! Execution Tauri commands (signatures in `src/domain/api.ts`): worktrees, the run queue,
//! launching, review, cancellation and diffs. Thin: parse, then `state.execution.<method>(…)`.
//! `open_in_editor`/`open_worktree` are the exception: they spawn processes (shell-only I/O),
//! reading the run and the editor setting through the `execution`/`board` facades.

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, State};

use nodal_app::execution::NOTE_APP_CLOSED;
use nodal_app::App;

use crate::commands::CommandError;
use crate::domain::*;
use crate::runs::claude_bin;
use crate::util::blocking;
use crate::work::diff::RunDiff;
use crate::work::dto::{LaunchInput, MergeInput};
use crate::work::merge::MergeReport;
use crate::work::queue::WorkSummary;
use crate::work::validate;
use crate::work::worktree::WorktreeStatus;

const OPEN_TIMEOUT: Duration = Duration::from_secs(3);

/// `App::run_pump`'s queue loop needs the runs left `launching` from a previous session
/// closed first (today's first step of `work::init`). `db` is unused now: `commands::mcp`/
/// `mcp::server` moved to `Arc<App>` (agent_api, merged concurrently), so this no longer needs
/// to keep the legacy `WorkState`/`Inner` alive for them.
pub fn setup(h: &AppHandle, app: &Arc<App>, _db: &crate::db::Db) -> Result<(), String> {
    // A `launching` from a previous session may or may not have actually launched. Like
    // today, a failure here is only logged: the pump and the MCP socket still start.
    if let Err(e) = app.execution.fail_stale_launches(NOTE_APP_CLOSED) {
        eprintln!("work: {e}");
    }
    tauri::async_runtime::spawn(app.clone().run_pump());
    crate::commands::mcp::setup(h, app);
    Ok(())
}

#[tauri::command]
pub async fn worktree_status(state: State<'_, Arc<App>>, task_id: String) -> Result<WorktreeStatus, CommandError> {
    Ok(state.execution.worktree_status(task_id).await?)
}

/// "Clean up": deletes the task's worktree and branch. Refuses with runs in progress and,
/// without `force`, if there are unpushed commits or uncommitted changes.
#[tauri::command]
pub async fn cleanup_worktree(state: State<'_, Arc<App>>, task_id: String, force: Option<bool>) -> Result<Task, CommandError> {
    Ok(state.execution.cleanup_worktree(task_id, force).await?)
}

/// "Merge into <base> & done": lands the task branch on its base, marks the task Done and, if
/// asked, pushes the base and cleans up the worktree.
#[tauri::command]
pub async fn merge_worktree(state: State<'_, Arc<App>>, task_id: String, input: MergeInput) -> Result<MergeReport, CommandError> {
    Ok(state.execution.merge_worktree(task_id, input).await?)
}

/// `project_id` (optional) filters by project in addition to task.
#[tauri::command]
pub async fn list_task_runs(
    state: State<'_, Arc<App>>,
    task_id: Option<String>,
    project_id: Option<String>,
) -> Result<Vec<Run>, CommandError> {
    Ok(state.execution.list_task_runs(task_id, project_id).await?)
}

/// Like `list_task_runs`, without `prompt` or `extraInstructions`.
#[tauri::command]
pub async fn list_runs_light(
    state: State<'_, Arc<App>>,
    project_id: Option<String>,
    task_id: Option<String>,
) -> Result<Vec<RunLight>, CommandError> {
    Ok(state.execution.list_runs_light(project_id, task_id).await?)
}

#[tauri::command]
pub async fn get_run(state: State<'_, Arc<App>>, run_id: String) -> Result<Run, CommandError> {
    Ok(state.execution.get_run(run_id).await?)
}

/// The last run of each task (no history limit), lightweight.
#[tauri::command]
pub async fn latest_runs_by_task(state: State<'_, Arc<App>>, project_id: Option<String>) -> Result<Vec<RunLight>, CommandError> {
    Ok(state.execution.latest_runs_by_task(project_id).await?)
}

/// Occupied slots/capacity, queue and what's waiting on the user (`project_id` null:
/// everything, including foreign sessions). If `claude agents` fails, it's computed without
/// the live sessions.
#[tauri::command]
pub async fn work_summary(state: State<'_, Arc<App>>, project_id: Option<String>) -> Result<WorkSummary, CommandError> {
    Ok(state.execution.work_summary(project_id).await?)
}

#[tauri::command]
pub async fn list_queue(state: State<'_, Arc<App>>) -> Result<Vec<Run>, CommandError> {
    Ok(state.execution.list_queue().await?)
}

#[tauri::command]
pub async fn launch_task(state: State<'_, Arc<App>>, task_id: String, input: Option<LaunchInput>) -> Result<Run, CommandError> {
    Ok(state.execution.launch_task(task_id, input).await?)
}

#[tauri::command]
pub async fn hand_off(
    state: State<'_, Arc<App>>,
    task_id: String,
    executor: Executor,
    extra_instructions: Option<String>,
) -> Result<Run, CommandError> {
    Ok(state.execution.hand_off(task_id, executor, extra_instructions).await?)
}

#[tauri::command]
pub async fn review_now(state: State<'_, Arc<App>>, task_id: String, reviewer: Option<String>) -> Result<Run, CommandError> {
    Ok(state.execution.review_now(task_id, reviewer).await?)
}

/// Confirms a migrated run that was left queued (it doesn't launch on its own).
#[tauri::command]
pub async fn confirm_run(state: State<'_, Arc<App>>, run_id: String) -> Result<Run, CommandError> {
    Ok(state.execution.confirm_run(run_id).await?)
}

/// Dequeues a `queued` run, or stops a launched one: saves whatever it left half-done as a
/// patch and the task moves to Blocked.
#[tauri::command]
pub async fn cancel_run(state: State<'_, Arc<App>>, run_id: String) -> Result<Run, CommandError> {
    Ok(state.execution.cancel_run(run_id).await?)
}

#[tauri::command]
pub async fn reorder_queue(state: State<'_, Arc<App>>, run_ids: Vec<String>) -> Result<(), CommandError> {
    Ok(state.execution.reorder_queue(run_ids).await?)
}

#[tauri::command]
pub async fn run_diff(state: State<'_, Arc<App>>, run_id: String) -> Result<RunDiff, CommandError> {
    Ok(state.execution.run_diff(run_id).await?)
}

/// Spawns the process and waits a bit: if it exits with an error right away, that's
/// reported; if it keeps running (some editor CLIs don't return), it's left alone and reaped
/// in the background.
async fn spawn_open(mut cmd: tokio::process::Command, what: &str) -> Result<(), String> {
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("Couldn't run {what}: {e}"))?;
    match tokio::time::timeout(OPEN_TIMEOUT, child.wait()).await {
        Ok(Ok(s)) if s.success() => Ok(()),
        Ok(Ok(s)) => {
            let mut err = String::new();
            if let Some(mut e) = child.stderr.take() {
                use tokio::io::AsyncReadExt;
                let _ = e.read_to_string(&mut err).await;
            }
            Err(format!("{what} failed ({s}): {}", err.trim()))
        }
        Ok(Err(e)) => Err(format!("{what} failed: {e}")),
        Err(_) => {
            tauri::async_runtime::spawn(async move {
                let _ = child.wait().await;
            });
            Ok(())
        }
    }
}

/// Opens the run's folder in Finder.
#[tauri::command]
pub async fn open_worktree(state: State<'_, Arc<App>>, run_id: String) -> Result<(), CommandError> {
    if !cfg!(target_os = "macos") {
        return Err("Opening Finder is only available on macOS.".to_string().into());
    }
    let dir = state.execution.run_cwd(run_id).await?;
    let mut cmd = tokio::process::Command::new("/usr/bin/open");
    cmd.arg("--").arg(dir);
    Ok(spawn_open(cmd, "open").await?)
}

/// Opens the run's folder (or `file` inside it) in the Settings editor. Without one set,
/// uses the first known editor whose CLI is installed, and only then the system text editor.
#[tauri::command]
pub async fn open_in_editor(state: State<'_, Arc<App>>, run_id: String, file: Option<String>) -> Result<(), CommandError> {
    let dir = state.execution.run_cwd(run_id).await?;
    let target = match file.as_deref().map(str::trim).filter(|f| !f.is_empty()) {
        Some(f) => {
            let (dir, f) = (dir.clone(), f.to_string());
            Some(
                blocking(move || {
                    let canon = dir.join(&f).canonicalize().map_err(|_| format!("The file doesn't exist: {f}"))?;
                    let root = dir.canonicalize().map_err(|e| e.to_string())?;
                    if !canon.starts_with(&root) {
                        return Err("The file must be inside the run's folder.".into());
                    }
                    Ok(canon)
                })
                .await
                .map_err(CommandError::from)?,
            )
        }
        None => None,
    };
    // (binary, name for errors)
    let editor = match state.board.get_settings().await?.editor {
        Some(e) => {
            let e = validate::editor(&e).map_err(CommandError::from)?;
            let bin = claude_bin::resolve_bin(&e).ok_or_else(|| CommandError::from(format!("Couldn't find `{e}` in PATH.")))?;
            Some((bin, format!("`{e}`")))
        }
        None => detect_editor().map(|(e, bin)| (bin, format!("`{e}` (picked automatically; choose one in Settings)"))),
    };
    let what = editor.as_ref().map_or_else(|| "the system text editor".to_string(), |(_, w)| w.clone());
    let mut cmd = match editor {
        Some((bin, _)) => {
            let mut cmd = tokio::process::Command::new(bin);
            cmd.env("PATH", claude_bin::augmented_path());
            cmd.arg(&dir);
            cmd
        }
        None => {
            if !cfg!(target_os = "macos") {
                return Err("Set an editor in Settings.".to_string().into());
            }
            // `-t`: always as text. Without it, a `.command` or `.app` left by the agent would
            // be executed. The folder opens in Finder.
            let mut cmd = tokio::process::Command::new("/usr/bin/open");
            if target.is_some() {
                cmd.arg("-t");
            }
            cmd.arg("--");
            if target.is_none() {
                cmd.arg(&dir);
            }
            cmd
        }
    };
    if let Some(t) = target {
        cmd.arg(t);
    }
    Ok(spawn_open(cmd, &what).await?)
}

/// First code editor from `validate::EDITORS` with its CLI installed, and where it is. Full IDEs
/// (`idea`, `webstorm`, `fleet`) are left out: too heavy to open just to look at a file.
fn detect_editor() -> Option<(&'static str, std::path::PathBuf)> {
    validate::EDITORS
        .iter()
        .filter(|e| !matches!(**e, "idea" | "webstorm" | "fleet"))
        .find_map(|e| claude_bin::resolve_bin(e).map(|bin| (*e, bin)))
}
