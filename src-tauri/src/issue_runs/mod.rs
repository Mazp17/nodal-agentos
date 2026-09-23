//! Runs lanzados desde las issues del board: catálogo de workflows, asociación
//! issue ↔ run persistida, cola con límite de concurrencia, attach y stop.
//!
//! La cola vive en `store.rs` (datos y lógica pura); acá están los efectos. Un task en
//! background la revisa cada `TICK` y lanza lo encolado cuando hay slots libres.

mod store;
pub mod workflows;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Manager, State};
use tokio::sync::Mutex;

use crate::config;
use crate::runs::{self, claude_bin, claude_fs};
pub use store::{IssueRun, IssueRunStatus};
use store::StoreData;
use workflows::WorkflowInfo;

const TICK: Duration = Duration::from_secs(5);
const STOP_TIMEOUT: Duration = Duration::from_secs(20);
const OSASCRIPT_TIMEOUT: Duration = Duration::from_secs(15);

struct Inner {
    store_path: PathBuf,
    config_path: PathBuf,
    data: Mutex<StoreData>,
    /// Serializa las pasadas de la cola: una sola a la vez decide y lanza.
    pump: Mutex<()>,
}

pub struct IssueRunsState(Arc<Inner>);

/// Carga la cola persistida, la registra como estado y arranca el task que la procesa.
pub fn init(app: &AppHandle) -> Result<(), String> {
    let store_path = app
        .path()
        .app_data_dir()
        .map(|d| d.join(store::FILE_NAME))
        .map_err(|e| format!("No se encontró la carpeta de datos de la app: {e}"))?;
    let config_path = config::config_path(app)?;
    let data = match store::load_from(&store_path) {
        Ok(d) => d,
        Err(e) => {
            // No perder el archivo: se aparta y se arranca vacío.
            let backup = store_path.with_extension(format!("json.corrupt-{}", store::now_ms()));
            let _ = std::fs::rename(&store_path, &backup);
            eprintln!("issue-runs: {e}; se movió a {}", backup.display());
            StoreData::default()
        }
    };
    let inner = Arc::new(Inner { store_path, config_path, data: Mutex::new(data), pump: Mutex::new(()) });
    app.manage(IssueRunsState(inner.clone()));
    tauri::async_runtime::spawn(async move {
        loop {
            if let Err(e) = pump(&inner).await {
                eprintln!("issue-runs: {e}");
            }
            tokio::time::sleep(TICK).await;
        }
    });
    Ok(())
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("Fallo interno: {e}"))?
}

async fn persist(inner: &Inner, data: &StoreData) -> Result<(), String> {
    let path = inner.store_path.clone();
    let data = data.clone();
    blocking(move || store::save_to(&path, &data)).await
}

async fn load_config(inner: &Inner) -> Result<config::AppConfig, String> {
    let path = inner.config_path.clone();
    blocking(move || config::load_from(&path)).await
}

/// Una pasada de la cola: completa sessionIds y lanza lo encolado que entre.
/// Si no hay nada pendiente no consulta `claude agents`.
async fn pump(inner: &Inner) -> Result<(), String> {
    let _turn = inner.pump.lock().await;
    if !store::needs_tick(&inner.data.lock().await.runs, store::now_ms()) {
        return Ok(());
    }
    let live = runs::list_runs().await?;
    let concurrency = load_config(inner).await.map(|c| c.concurrency).unwrap_or(config::DEFAULT_CONCURRENCY);

    let to_launch: Vec<IssueRun> = {
        let mut data = inner.data.lock().await;
        let mut changed = store::fill_session_ids(&mut data.runs, &live);
        let picked = store::next_to_launch(&data.runs, &live, concurrency, store::now_ms());
        for &i in &picked {
            data.runs[i].status = IssueRunStatus::Launching;
            changed = true;
        }
        if changed {
            persist(inner, &data).await?;
        }
        picked.iter().map(|&i| data.runs[i].clone()).collect()
    };

    for run in to_launch {
        let result = runs::launch_run(run.cwd.clone(), run.prompt()).await;
        let mut data = inner.data.lock().await;
        let entry = data.runs.iter_mut().find(|r| {
            r.issue_id == run.issue_id && r.queued_at == run.queued_at && r.status == IssueRunStatus::Launching
        });
        if let Some(e) = entry {
            match result {
                Ok(r) => {
                    e.run_id = Some(r.id);
                    e.launched_at = Some(store::now_ms());
                    e.status = IssueRunStatus::Launched;
                }
                Err(err) => {
                    e.status = IssueRunStatus::Failed;
                    e.error = Some(err);
                }
            }
        }
        persist(inner, &data).await?;
    }
    Ok(())
}

fn user_workflows_dir() -> Option<PathBuf> {
    claude_fs::claude_config_dir().map(|d| d.join("workflows"))
}

/// Identificador de Linear ("ACME-8"): termina en el prompt, así que nada raro.
fn is_valid_identifier(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 40
        && !id.starts_with('-')
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Id corto de un run (`claude --bg` imprime hex). Va a un comando y a AppleScript.
pub fn is_valid_run_id(id: &str) -> bool {
    (4..=64).contains(&id.len()) && !id.starts_with('-') && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

#[tauri::command]
pub async fn list_workflows(repo_path: Option<String>) -> Result<Vec<WorkflowInfo>, String> {
    let repo = repo_path.map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
    if let Some(r) = &repo {
        if !Path::new(r).is_absolute() {
            return Err(format!("La ruta del repo tiene que ser absoluta: {r}"));
        }
    }
    blocking(move || Ok(workflows::list_from(user_workflows_dir().as_deref(), repo.as_deref().map(Path::new)))).await
}

#[tauri::command]
pub async fn list_issue_runs(state: State<'_, IssueRunsState>) -> Result<Vec<IssueRun>, String> {
    let mut runs = state.0.data.lock().await.runs.clone();
    runs.sort_by_key(|r| std::cmp::Reverse(r.queued_at));
    Ok(runs)
}

/// Encola un run del workflow para la issue y dispara una pasada de la cola: si hay
/// slot, sale lanzado; si no, queda `queued`.
#[tauri::command]
pub async fn launch_issue_run(
    state: State<'_, IssueRunsState>,
    issue_id: String,
    identifier: String,
    team_id: String,
    project_id: Option<String>,
    workflow: String,
) -> Result<IssueRun, String> {
    let inner = state.0.clone();
    let identifier = identifier.trim().to_string();
    let workflow = workflow.trim().to_string();
    if issue_id.trim().is_empty() {
        return Err("Falta el id de la issue.".into());
    }
    if !is_valid_identifier(&identifier) {
        return Err(format!("Identificador de issue inválido: «{identifier}»."));
    }
    if !workflows::is_valid_workflow_name(&workflow) {
        return Err(format!("Nombre de workflow inválido: «{workflow}»."));
    }

    let cfg = load_config(&inner).await?;
    let project_id = project_id.filter(|p| !p.trim().is_empty());
    let repo = config::resolve(&cfg, &team_id, project_id.as_deref()).ok_or_else(|| {
        format!("{identifier} no tiene un repo mapeado para su team/proyecto. Configuralo en Ajustes.")
    })?;
    let (repo_ok, catalog) = {
        let repo = repo.clone();
        blocking(move || {
            let ok = Path::new(&repo).is_dir();
            Ok((ok, workflows::list_from(user_workflows_dir().as_deref(), Some(Path::new(&repo)))))
        })
        .await?
    };
    if !repo_ok {
        return Err(format!("La carpeta del repo mapeado no existe: {repo}"));
    }
    if !catalog.iter().any(|w| w.name == workflow) {
        return Err(format!(
            "No existe el workflow «{workflow}» ni en ~/.claude/workflows ni en {repo}/.claude/workflows."
        ));
    }

    // Un solo run activo por issue. Si `claude agents` falla, se decide con lo que hay.
    let live = runs::list_runs().await.ok();
    let entry = {
        let mut data = inner.data.lock().await;
        let now = store::now_ms();
        let current = data.runs.iter().filter(|r| r.issue_id == issue_id).max_by_key(|r| r.queued_at);
        if let Some(cur) = current {
            if store::is_active(cur, live.as_deref(), now) {
                let what = match cur.status {
                    IssueRunStatus::Queued => "ya está en cola",
                    IssueRunStatus::Launching => "se está lanzando",
                    _ => "ya tiene un run en curso",
                };
                return Err(format!("{identifier} {what}."));
            }
        }
        let entry = IssueRun {
            issue_id: issue_id.clone(),
            identifier,
            workflow,
            run_id: None,
            session_id: None,
            cwd: repo,
            queued_at: now,
            launched_at: None,
            status: IssueRunStatus::Queued,
            error: None,
        };
        data.runs.push(entry.clone());
        store::prune(&mut data);
        persist(&inner, &data).await?;
        entry
    };

    // Si la pasada falla (p. ej. `claude agents` no responde) el run queda en cola y el
    // task lo reintenta; no es un error del lanzamiento.
    if let Err(e) = pump(&inner).await {
        eprintln!("issue-runs: {e}");
    }
    let data = inner.data.lock().await;
    Ok(data
        .runs
        .iter()
        .find(|r| r.issue_id == entry.issue_id && r.queued_at == entry.queued_at)
        .cloned()
        .unwrap_or(entry))
}

/// Saca de la cola los runs pendientes de la issue.
#[tauri::command]
pub async fn cancel_queued(state: State<'_, IssueRunsState>, issue_id: String) -> Result<(), String> {
    let inner = state.0.clone();
    let mut data = inner.data.lock().await;
    if data.runs.iter().any(|r| r.issue_id == issue_id && r.status == IssueRunStatus::Launching) {
        return Err("El run ya se está lanzando; esperá a que arranque y detenelo con Stop.".into());
    }
    let before = data.runs.len();
    data.runs.retain(|r| !(r.issue_id == issue_id && r.status == IssueRunStatus::Queued));
    if data.runs.len() == before {
        return Err("Esa issue no tiene runs en cola.".into());
    }
    persist(&inner, &data).await
}

#[tauri::command]
pub async fn stop_run(run_id: String) -> Result<(), String> {
    if !is_valid_run_id(&run_id) {
        return Err(format!("Id de run inválido: «{run_id}»."));
    }
    let mut cmd = claude_bin::claude_command()?;
    cmd.args(["stop", &run_id]);
    let out = claude_bin::output_with_timeout(cmd, STOP_TIMEOUT, "`claude stop`").await?;
    if !out.status.success() {
        return Err(format!("`claude stop {run_id}` falló: {}", claude_bin::error_text(&out)));
    }
    Ok(())
}

/// Comillas simples de shell: `'...'` con `'` → `'\''`.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Literal de string AppleScript.
fn applescript_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', r"\\").replace('"', "\\\""))
}

/// Líneas `-e` para osascript: abre una ventana de Terminal con `claude attach <id>`.
/// `run_id` ya viene validado; el path del binario se cita igual por si tiene espacios.
fn attach_script(claude: &Path, run_id: &str) -> Vec<String> {
    let cmd = format!("{} attach {}", shell_quote(&claude.to_string_lossy()), run_id);
    vec![
        "tell application \"Terminal\"".into(),
        "activate".into(),
        format!("do script {}", applescript_string(&cmd)),
        "end tell".into(),
    ]
}

#[tauri::command]
pub async fn attach_run(run_id: String) -> Result<(), String> {
    if !is_valid_run_id(&run_id) {
        return Err(format!("Id de run inválido: «{run_id}»."));
    }
    if !cfg!(target_os = "macos") {
        return Err("Attach solo está disponible en macOS (usa Terminal.app).".into());
    }
    let claude = claude_bin::resolve_claude()?;
    let mut cmd = tokio::process::Command::new("/usr/bin/osascript");
    for line in attach_script(&claude, &run_id) {
        cmd.arg("-e").arg(line);
    }
    cmd.stdin(std::process::Stdio::null());
    let out = claude_bin::output_with_timeout(cmd, OSASCRIPT_TIMEOUT, "osascript").await?;
    if !out.status.success() {
        return Err(format!("No se pudo abrir Terminal: {}", claude_bin::error_text(&out)));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_id_validation() {
        assert!(is_valid_run_id("ddb91222"));
        assert!(is_valid_run_id("abc-123"));
        assert!(!is_valid_run_id("-rf"));
        assert!(!is_valid_run_id("ab"));
        assert!(!is_valid_run_id("abcd\"; rm -rf ~"));
        assert!(!is_valid_run_id("abcd efgh"));
        assert!(!is_valid_run_id("abcd'"));
    }

    #[test]
    fn identifier_validation() {
        assert!(is_valid_identifier("ACME-8"));
        assert!(!is_valid_identifier("JOI 8"));
        assert!(!is_valid_identifier("--help"));
        assert!(!is_valid_identifier(""));
    }

    #[test]
    fn attach_script_quotes_path() {
        let lines = attach_script(Path::new("/Users/a b/it's \"x\"/claude"), "ddb91222");
        assert_eq!(lines[0], "tell application \"Terminal\"");
        // Shell: '/Users/a b/it'\''s "x"/claude' attach ddb91222, luego escapado para AppleScript.
        assert_eq!(
            lines[2],
            r#"do script "'/Users/a b/it'\\''s \"x\"/claude' attach ddb91222""#
        );
    }
}
