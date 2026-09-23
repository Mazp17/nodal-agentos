//! Runs en background de Claude Code: lanzarlos (`claude --bg`), listarlos
//! (`claude agents`) y seguir el workflow que corren leyendo sus archivos de sesión.

pub mod claude_bin;
pub(crate) mod claude_fs;
pub mod options;
pub mod types;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tauri::AppHandle;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::mpsc;

use crate::config;
use types::{LaunchOptions, RunDetail, RunRef, RunSummary, Transcript};

const LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);
const LIST_TIMEOUT: Duration = Duration::from_secs(15);

/// Reenvía cada línea de un stream del hijo al canal.
fn forward_lines<R: AsyncRead + Unpin + Send + 'static>(stream: R, tx: mpsc::UnboundedSender<String>) {
    tauri::async_runtime::spawn(async move {
        let mut lines = BufReader::new(stream).lines();
        // Se drena hasta EOF aunque ya nadie escuche: cerrar el pipe le daría EPIPE a
        // `claude` (o a la sesión en background, si heredó el pipe).
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = tx.send(line);
        }
    });
}

/// Lanza `claude --bg [flags] <prompt>` en `cwd` y devuelve el id corto de la sesión.
/// Todo va como argumentos propios (sin shell), así que no hay nada que escapar; los
/// flags se validan contra las listas permitidas de `options`.
pub async fn launch_with(cwd: String, prompt: String, opts: &LaunchOptions) -> Result<RunRef, String> {
    let dir = Path::new(&cwd);
    if !dir.is_absolute() {
        return Err(format!("The folder must be an absolute path: {cwd}"));
    }
    if !dir.is_dir() {
        return Err(format!("The folder doesn't exist or isn't a directory: {cwd}"));
    }
    if prompt.trim().is_empty() {
        return Err("The prompt is empty.".into());
    }
    // `claude` lo interpretaría como una opción, no como prompt.
    if prompt.trim_start().starts_with('-') {
        return Err("The prompt can't start with \"-\".".into());
    }
    let flags = options::to_args(opts)?;

    let mut cmd = claude_bin::claude_command()?;
    cmd.arg("--bg")
        .args(&flags)
        .arg(&prompt)
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("Couldn't run `claude --bg`: {e}"))?;

    // No se espera a EOF: basta con ver la línea `backgrounded · <id>` en cualquiera de
    // los dos streams (no hay garantía de que la sesión en background suelte los pipes).
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
            // Cosechar el proceso cuando termine, sin bloquear la respuesta.
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
            Err(format!("`claude --bg` exited without returning the session id{code}: {seen}"))
        }
        Err(_) => {
            let _ = child.start_kill();
            Err(format!(
                "`claude --bg` didn't return the session id within {} s. It may have launched anyway: check the runs list before retrying.",
                LAUNCH_TIMEOUT.as_secs()
            ))
        }
    }
}

/// Lanzamiento manual: usa el model/effort/permission mode del repo si `cwd` está mapeado.
#[tauri::command]
pub async fn launch_run(app: AppHandle, cwd: String, prompt: String) -> Result<RunRef, String> {
    let path = config::config_path(&app)?;
    let key = cwd.clone();
    // Una config ilegible no bloquea el lanzamiento manual: se lanza sin flags.
    let opts = tauri::async_runtime::spawn_blocking(move || match config::load_from(&path) {
        Ok(cfg) => config::find_by_path(&cfg, &key).map(|m| m.launch_options()).unwrap_or_default(),
        Err(e) => {
            eprintln!("launch_run: {e}; launching without repo options");
            LaunchOptions::default()
        }
    })
    .await
    .map_err(|e| format!("Internal error: {e}"))?;
    launch_with(cwd, prompt, &opts).await
}

/// Sesiones en background (`claude agents --json --all`), más recientes primero.
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

/// Detalle del workflow más reciente de la sesión. `None` si la sesión todavía no tiene
/// carpeta en disco o no lanzó ningún workflow.
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

/// Transcript de un subagente de un workflow: prompt, conversación (recortada) y salida.
/// `limit`: cuántos items devolver como mucho, los más recientes (default 200, máx. 2000).
/// `None` si el agente todavía no tiene archivo.
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

    /// Contra el `claude` real y los datos de esta máquina: `cargo test -- --ignored`.
    /// Solo lee (`claude agents` + archivos); no lanza runs.
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
        let err = tauri::async_runtime::block_on(launch_with("/no/existe".into(), "x".into(), &LaunchOptions::default()))
            .unwrap_err();
        assert_eq!(err, "The folder doesn't exist or isn't a directory: /no/existe");
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
