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
        .map_err(|e| format!("Couldn't find the app data folder: {e}"))?;
    let config_path = config::config_path(app)?;
    let data = match store::load_from(&store_path) {
        Ok(d) => d,
        Err(e) => {
            // No perder el archivo: se aparta y se arranca vacío.
            let backup = store_path.with_extension(format!("json.corrupt-{}", store::now_ms()));
            let _ = std::fs::rename(&store_path, &backup);
            eprintln!("issue-runs: {e}; moved to {}", backup.display());
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
        .map_err(|e| format!("Internal error: {e}"))?
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
        // Un fallo de escritura no corta la pasada: la memoria manda y se reintenta
        // guardar en la próxima; cortar dejaría entradas colgadas en `launching`.
        if changed {
            if let Err(e) = persist(inner, &data).await {
                eprintln!("issue-runs: {e}");
            }
        }
        picked.iter().map(|&i| data.runs[i].clone()).collect()
    };

    for run in to_launch {
        let result = runs::launch_with(run.cwd.clone(), run.prompt(), &run.options).await;
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
        if let Err(e) = persist(inner, &data).await {
            eprintln!("issue-runs: {e}");
        }
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
            return Err(format!("The repo path must be absolute: {r}"));
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
        return Err("Missing issue id.".into());
    }
    if !is_valid_identifier(&identifier) {
        return Err(format!("Invalid issue identifier: \"{identifier}\"."));
    }
    if !workflows::is_valid_workflow_name(&workflow) {
        return Err(format!("Invalid workflow name: \"{workflow}\"."));
    }

    let cfg = load_config(&inner).await?;
    let project_id = project_id.filter(|p| !p.trim().is_empty());
    let mapping = config::resolve_mapping(&cfg, &team_id, project_id.as_deref()).ok_or_else(|| {
        format!("{identifier} has no repo mapped for its team/project. Set one up in Settings → Repos.")
    })?;
    let repo = mapping.path.clone();
    // Se validan ya (y no recién al lanzar) para que el error llegue a quien lanza.
    let options = runs::options::normalize(&mapping.launch_options())
        .map_err(|e| format!("The repo settings for {repo} are invalid:\n{}", e.join("\n")))?;
    let (repo_ok, catalog) = {
        let repo = repo.clone();
        blocking(move || {
            let ok = Path::new(&repo).is_dir();
            Ok((ok, workflows::list_from(user_workflows_dir().as_deref(), Some(Path::new(&repo)))))
        })
        .await?
    };
    if !repo_ok {
        return Err(format!("The mapped repo folder doesn't exist: {repo}"));
    }
    if !catalog.iter().any(|w| w.name == workflow) {
        return Err(format!(
            "Workflow \"{workflow}\" not found in ~/.claude/workflows or {repo}/.claude/workflows."
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
                    IssueRunStatus::Queued => "is already queued",
                    IssueRunStatus::Launching => "is launching",
                    _ => "already has a run in progress",
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
            options,
        };
        data.runs.push(entry.clone());
        if let Err(e) = persist(&inner, &data).await {
            // Sin persistir no se encola: si no, el tick lo lanzaría igual tras el error.
            data.runs.retain(|r| !(r.issue_id == entry.issue_id && r.queued_at == entry.queued_at));
            return Err(e);
        }
        store::prune(&mut data);
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
        return Err("The run is already launching; wait for it to start and stop it with Stop.".into());
    }
    let before = data.runs.len();
    data.runs.retain(|r| !(r.issue_id == issue_id && r.status == IssueRunStatus::Queued));
    if data.runs.len() == before {
        return Err("That issue has no queued runs.".into());
    }
    persist(&inner, &data).await
}

#[tauri::command]
pub async fn stop_run(run_id: String) -> Result<(), String> {
    if !is_valid_run_id(&run_id) {
        return Err(format!("Invalid run id: \"{run_id}\"."));
    }
    let mut cmd = claude_bin::claude_command()?;
    cmd.args(["stop", &run_id]);
    let out = claude_bin::output_with_timeout(cmd, STOP_TIMEOUT, "`claude stop`").await?;
    if !out.status.success() {
        return Err(format!("`claude stop {run_id}` failed: {}", claude_bin::error_text(&out)));
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
        return Err(format!("Invalid run id: \"{run_id}\"."));
    }
    if !cfg!(target_os = "macos") {
        return Err("Attach is only available on macOS (it uses Terminal.app).".into());
    }
    let claude = claude_bin::resolve_claude()?;
    let mut cmd = tokio::process::Command::new("/usr/bin/osascript");
    for line in attach_script(&claude, &run_id) {
        cmd.arg("-e").arg(line);
    }
    cmd.stdin(std::process::Stdio::null());
    let out = claude_bin::output_with_timeout(cmd, OSASCRIPT_TIMEOUT, "osascript").await?;
    if !out.status.success() {
        return Err(format!("Couldn't open Terminal: {}", claude_bin::error_text(&out)));
    }
    Ok(())
}

/// Líneas `-e` para osascript: abre una ventana de Terminal en `dir` y, si viene
/// `claude`, lo arranca ahí (para aceptar el diálogo de confianza del workspace).
fn terminal_at_script(dir: &Path, claude: Option<&Path>) -> Vec<String> {
    let mut cmd = format!("cd {}", shell_quote(&dir.to_string_lossy()));
    if let Some(c) = claude {
        cmd.push_str(" && ");
        cmd.push_str(&shell_quote(&c.to_string_lossy()));
    }
    vec![
        "tell application \"Terminal\"".into(),
        "activate".into(),
        format!("do script {}", applescript_string(&cmd)),
        "end tell".into(),
    ]
}

/// Abre Terminal.app en `path` (un directorio existente). Con `run_claude`, arranca
/// `claude` ahí: sirve para aceptar el diálogo de confianza de un repo nuevo.
#[tauri::command]
pub async fn open_terminal_at(path: String, run_claude: Option<bool>) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("Opening Terminal is only available on macOS.".into());
    }
    let raw = Path::new(path.trim());
    if !raw.is_absolute() {
        return Err(format!("The folder must be an absolute path: {path}"));
    }
    // Un salto de línea (u otro control) partiría la línea de AppleScript.
    if path.chars().any(char::is_control) {
        return Err("The folder path contains control characters.".into());
    }
    let dir = raw
        .canonicalize()
        .ok()
        .filter(|d| d.is_dir())
        .ok_or_else(|| format!("The folder doesn't exist or isn't a directory: {path}"))?;
    let claude = if run_claude.unwrap_or(false) { Some(claude_bin::resolve_claude()?) } else { None };
    // `canonicalize` resuelve symlinks: se revisa también la ruta final (y la de claude).
    let has_control = |p: &Path| p.to_string_lossy().chars().any(char::is_control);
    if has_control(&dir) || claude.as_deref().is_some_and(has_control) {
        return Err("The folder path contains control characters.".into());
    }
    let mut cmd = tokio::process::Command::new("/usr/bin/osascript");
    for line in terminal_at_script(&dir, claude.as_deref()) {
        cmd.arg("-e").arg(line);
    }
    cmd.stdin(std::process::Stdio::null());
    let out = claude_bin::output_with_timeout(cmd, OSASCRIPT_TIMEOUT, "osascript").await?;
    if !out.status.success() {
        return Err(format!("Couldn't open Terminal: {}", claude_bin::error_text(&out)));
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

    #[test]
    fn terminal_at_script_quotes_path() {
        let lines = terminal_at_script(Path::new("/Users/a b/it's \"x\""), Some(Path::new("/bin/claude")));
        assert_eq!(lines[2], r#"do script "cd '/Users/a b/it'\\''s \"x\"' && '/bin/claude'""#);
        let lines = terminal_at_script(Path::new("/tmp"), None);
        assert_eq!(lines[2], r#"do script "cd '/tmp'""#);
    }
}
