//! Runs en background de Claude Code: lanzarlos (`claude --bg`), listarlos
//! (`claude agents`) y seguir el workflow que corren leyendo sus archivos de sesión.

pub mod claude_bin;
mod claude_fs;
pub mod types;

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::mpsc;

use types::{RunDetail, RunRef, RunSummary};

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

/// Lanza `claude --bg <prompt>` en `cwd` y devuelve el id corto de la sesión.
/// El prompt va como argumento propio (sin shell), así que no hay nada que escapar.
#[tauri::command]
pub async fn launch_run(cwd: String, prompt: String) -> Result<RunRef, String> {
    let dir = Path::new(&cwd);
    if !dir.is_absolute() {
        return Err(format!("La carpeta tiene que ser una ruta absoluta: {cwd}"));
    }
    if !dir.is_dir() {
        return Err(format!("La carpeta no existe o no es un directorio: {cwd}"));
    }
    if prompt.trim().is_empty() {
        return Err("El prompt está vacío.".into());
    }
    // `claude` lo interpretaría como una opción, no como prompt.
    if prompt.trim_start().starts_with('-') {
        return Err("El prompt no puede empezar con «-».".into());
    }

    let mut cmd = claude_bin::claude_command()?;
    cmd.arg("--bg")
        .arg(&prompt)
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("No se pudo ejecutar `claude --bg`: {e}"))?;

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
                Ok(Ok(s)) => s.code().map(|c| format!(" (código {c})")).unwrap_or_default(),
                _ => String::new(),
            };
            let seen: String = seen.trim().chars().take(500).collect();
            Err(format!("`claude --bg` terminó sin devolver el id de la sesión{code}: {seen}"))
        }
        Err(_) => {
            let _ = child.start_kill();
            Err(format!(
                "`claude --bg` no devolvió el id de la sesión en {} s. Puede haberse lanzado igual: revisá la lista antes de reintentar.",
                LAUNCH_TIMEOUT.as_secs()
            ))
        }
    }
}

/// Sesiones en background (`claude agents --json --all`), más recientes primero.
#[tauri::command]
pub async fn list_runs() -> Result<Vec<RunSummary>, String> {
    let mut cmd = claude_bin::claude_command()?;
    cmd.args(["agents", "--json", "--all"]);
    let out = claude_bin::output_with_timeout(cmd, LIST_TIMEOUT, "`claude agents`").await?;
    if !out.status.success() {
        return Err(format!("`claude agents` falló: {}", claude_bin::error_text(&out)));
    }
    claude_fs::parse_agents_json(&String::from_utf8_lossy(&out.stdout))
}

/// Detalle del workflow más reciente de la sesión. `None` si la sesión todavía no tiene
/// carpeta en disco o no lanzó ningún workflow.
#[tauri::command]
pub async fn get_run_detail(session_id: String, cwd: String) -> Result<Option<RunDetail>, String> {
    if !claude_fs::is_valid_session_id(&session_id) {
        return Err(format!("Id de sesión inválido: {session_id}"));
    }
    tauri::async_runtime::spawn_blocking(move || -> Result<Option<RunDetail>, String> {
        let projects = claude_fs::claude_config_dir()
            .ok_or("No se pudo ubicar la carpeta de Claude Code (falta $HOME).")?
            .join("projects");
        Ok(claude_fs::find_session_dir(&projects, &cwd, &session_id)
            .and_then(|dir| claude_fs::read_run_detail(&dir)))
    })
    .await
    .map_err(|e| format!("Error interno leyendo la sesión: {e}"))?
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
        eprintln!("{} runs en background", runs.len());
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
        let err = tauri::async_runtime::block_on(launch_run("/no/existe".into(), "x".into())).unwrap_err();
        assert_eq!(err, "La carpeta no existe o no es un directorio: /no/existe");
    }
}
