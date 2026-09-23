//! Tareas locales: un título + un plan (texto o `.md` del repo) que se ejecuta con el
//! workflow `plan-task` en un repo, sin pasar por Linear.
//!
//! El estado de ejecución NO se guarda en la tarea: se deriva del último `TaskRun`
//! (cola propia, mismo esquema que `issue_runs`) cruzado con `claude agents`.
//!
//! Unificación pendiente con `issue_runs`: su cola (`store.rs`) es privada y está atada a
//! `issue_id`/`identifier`/`workflow`, así que acá hay un store análogo. El refactor
//! recomendado es generalizar `IssueRun` a un `WorkItemRun { item: WorkItemRef, prompt, … }`
//! con `WorkItemRef::Issue{…} | Task{…}`, una sola cola/pump y un solo archivo; entonces
//! este módulo solo aporta validación, el store de tareas y el armado del prompt.

mod store;
pub mod validate;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Manager, State};
use tokio::sync::Mutex;

use crate::config;
use crate::runs;
pub use store::{PlanInput, PlanRef, Task, TaskRun, TaskRunStatus, TaskStatus};
use store::{RunsData, TasksData};

const TICK: Duration = Duration::from_secs(5);

struct Inner {
    /// Para `runs::launch_run`, que lee las opciones del repo (model, effort…) de la config.
    app: AppHandle,
    tasks_path: PathBuf,
    runs_path: PathBuf,
    plans_dir: PathBuf,
    config_path: PathBuf,
    /// Orden de locks cuando hacen falta los dos: `tasks` y después `runs`.
    tasks: Mutex<TasksData>,
    runs: Mutex<RunsData>,
    /// Serializa las pasadas de la cola.
    pump: Mutex<()>,
}

pub struct TasksState(Arc<Inner>);

impl TasksState {
    /// `(run_id, session_id)` de todos los runs lanzados por tareas (para marcar
    /// "lanzado por la app" en la actividad del repo).
    pub async fn launched_refs(&self) -> Vec<(Option<String>, Option<String>)> {
        self.0.runs.lock().await.runs.iter().map(|r| (r.run_id.clone(), r.session_id.clone())).collect()
    }
}

fn load_or_backup<T: Default>(path: &Path, load: impl Fn(&Path) -> Result<T, String>) -> Result<T, String> {
    match load(path) {
        Ok(d) => Ok(d),
        // Un error de lectura (permisos, E/S) no es corrupción: no se aparta el archivo.
        Err(e) if !e.contains("is corrupt") => Err(e),
        Err(e) => {
            // No perder el archivo: se aparta y se arranca vacío.
            let backup = path.with_extension(format!("json.corrupt-{}", store::now_ms()));
            let _ = std::fs::rename(path, &backup);
            eprintln!("tasks: {e}; moved to {}", backup.display());
            Ok(T::default())
        }
    }
}

pub fn init(app: &AppHandle) -> Result<(), String> {
    let data_dir = app.path().app_data_dir().map_err(|e| format!("Couldn't find the app data folder: {e}"))?;
    let tasks_path = data_dir.join(store::TASKS_FILE);
    let runs_path = data_dir.join(store::RUNS_FILE);
    let tasks = load_or_backup(&tasks_path, store::load_tasks)?;
    let runs = load_or_backup(&runs_path, store::load_runs)?;
    let inner = Arc::new(Inner {
        app: app.clone(),
        tasks_path,
        runs_path,
        plans_dir: data_dir.join(store::PLANS_DIR),
        config_path: config::config_path(app)?,
        tasks: Mutex::new(tasks),
        runs: Mutex::new(runs),
        pump: Mutex::new(()),
    });
    app.manage(TasksState(inner.clone()));
    tauri::async_runtime::spawn(async move {
        loop {
            if let Err(e) = pump(&inner).await {
                eprintln!("tasks: {e}");
            }
            tokio::time::sleep(TICK).await;
        }
    });
    Ok(())
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| format!("Internal error: {e}"))?
}

async fn persist_tasks(inner: &Inner, data: &TasksData) -> Result<(), String> {
    let (path, data) = (inner.tasks_path.clone(), data.clone());
    blocking(move || store::save_tasks(&path, &data)).await
}

async fn persist_runs(inner: &Inner, data: &RunsData) -> Result<(), String> {
    let (path, data) = (inner.runs_path.clone(), data.clone());
    blocking(move || store::save_runs(&path, &data)).await
}

fn plan_dir(inner: &Inner, id: &str) -> PathBuf {
    inner.plans_dir.join(id)
}

fn text_plan_path(inner: &Inner, id: &str) -> PathBuf {
    plan_dir(inner, id).join(store::PLAN_FILE)
}

fn check_id(id: &str) -> Result<(), String> {
    if store::is_valid_task_id(id) {
        Ok(())
    } else {
        Err(format!("Invalid task id: «{id}»."))
    }
}

fn find_task(data: &TasksData, id: &str) -> Result<Task, String> {
    data.tasks.iter().find(|t| t.id == id).cloned().ok_or_else(|| "That task no longer exists.".into())
}

/// Una pasada de la cola: completa sessionIds y lanza lo encolado que entre.
async fn pump(inner: &Inner) -> Result<(), String> {
    let _turn = inner.pump.lock().await;
    if !store::needs_tick(&inner.runs.lock().await.runs, store::now_ms()) {
        return Ok(());
    }
    let live = runs::list_runs().await?;
    let path = inner.config_path.clone();
    let concurrency = blocking(move || config::load_from(&path))
        .await
        .map(|c| c.concurrency)
        .unwrap_or(config::DEFAULT_CONCURRENCY);

    let to_launch: Vec<TaskRun> = {
        let mut data = inner.runs.lock().await;
        let mut changed = store::fill_session_ids(&mut data.runs, &live);
        let picked = store::next_to_launch(&data.runs, &live, concurrency, store::now_ms());
        for &i in &picked {
            data.runs[i].status = TaskRunStatus::Launching;
            changed = true;
        }
        if changed {
            if let Err(e) = persist_runs(inner, &data).await {
                eprintln!("tasks: {e}");
            }
        }
        picked.iter().map(|&i| data.runs[i].clone()).collect()
    };

    for run in to_launch {
        let result = runs::launch_run(inner.app.clone(), run.cwd.clone(), run.prompt.clone()).await;
        let mut data = inner.runs.lock().await;
        let entry = data.runs.iter_mut().find(|r| {
            r.task_id == run.task_id && r.queued_at == run.queued_at && r.status == TaskRunStatus::Launching
        });
        if let Some(e) = entry {
            match result {
                Ok(r) => {
                    e.run_id = Some(r.id);
                    e.launched_at = Some(store::now_ms());
                    e.status = TaskRunStatus::Launched;
                }
                Err(err) => {
                    e.status = TaskRunStatus::Failed;
                    e.error = Some(err);
                }
            }
        }
        if let Err(e) = persist_runs(inner, &data).await {
            eprintln!("tasks: {e}");
        }
    }
    Ok(())
}

/// Valida el plan y, si es texto, lo escribe en `path` (el definitivo al crear, o un
/// archivo de staging al editar, que se renombra recién cuando se guardó `tasks.json`).
fn apply_plan(repo: &Path, plan: PlanInput, path: &Path) -> Result<PlanRef, String> {
    match plan {
        PlanInput::Text { text } => {
            validate::plan_text(&text)?;
            store::write_atomic(path, text.as_bytes())?;
            Ok(PlanRef::Text)
        }
        PlanInput::File { path } => {
            let canon = validate::plan_file(repo, &path)?;
            Ok(PlanRef::File { path: canon.to_string_lossy().into_owned() })
        }
    }
}

#[tauri::command]
pub async fn create_task(
    state: State<'_, TasksState>,
    repo_path: String,
    title: String,
    plan: PlanInput,
) -> Result<Task, String> {
    let inner = state.0.clone();
    let title = validate::title(&title)?;
    let now = store::now_ms();
    let id = store::new_id(now);
    let text_path = text_plan_path(&inner, &id);
    let (repo, plan_ref) = {
        let text_path = text_path.clone();
        blocking(move || {
            let repo = validate::repo_root(&repo_path)?;
            let plan = apply_plan(&repo, plan, &text_path)?;
            Ok((repo, plan))
        })
        .await?
    };
    let task = Task {
        id: id.clone(),
        repo_path: repo.to_string_lossy().into_owned(),
        title,
        plan: plan_ref,
        status: TaskStatus::Todo,
        created_at: now,
        done_at: None,
    };
    let mut data = inner.tasks.lock().await;
    data.tasks.push(task.clone());
    if let Err(e) = persist_tasks(&inner, &data).await {
        data.tasks.retain(|t| t.id != id);
        let dir = plan_dir(&inner, &id);
        let _ = blocking(move || std::fs::remove_dir_all(dir).or(Ok(()))).await;
        return Err(e);
    }
    Ok(task)
}

/// Cambia título y/o plan. El repo no se cambia (sería otra tarea).
#[tauri::command]
pub async fn update_task(
    state: State<'_, TasksState>,
    id: String,
    title: Option<String>,
    plan: Option<PlanInput>,
) -> Result<Task, String> {
    check_id(&id)?;
    let inner = state.0.clone();
    let title = title.map(|t| validate::title(&t)).transpose()?;
    let mut data = inner.tasks.lock().await;
    let old = find_task(&data, &id)?;
    let mut task = old.clone();
    let text_path = text_plan_path(&inner, &id);
    let staged = text_path.with_extension("md.new");
    if let Some(plan) = plan {
        let repo = task.repo_path.clone();
        let staged = staged.clone();
        task.plan = blocking(move || {
            let repo = validate::repo_root(&repo)?;
            apply_plan(&repo, plan, &staged)
        })
        .await?;
    }
    if let Some(t) = title {
        task.title = t;
    }
    if let Some(slot) = data.tasks.iter_mut().find(|t| t.id == id) {
        *slot = task.clone();
    }
    let saved = persist_tasks(&inner, &data).await;
    let ok = saved.is_ok();
    let new_is_text = task.plan == PlanRef::Text;
    let was_text = old.plan == PlanRef::Text;
    // Recién ahora se tocan los archivos del plan: si el guardado falló, quedan como estaban.
    let files = blocking(move || {
        if !ok {
            let _ = std::fs::remove_file(&staged);
            return Ok(());
        }
        if staged.is_file() {
            std::fs::rename(&staged, &text_path).map_err(|e| format!("Couldn't save the plan: {e}"))?;
        } else if was_text && !new_is_text {
            let _ = std::fs::remove_file(&text_path);
        }
        Ok(())
    })
    .await;
    if let Err(e) = saved {
        if let Some(slot) = data.tasks.iter_mut().find(|t| t.id == id) {
            *slot = old;
        }
        return Err(e);
    }
    files?;
    Ok(task)
}

/// Borra la tarea, su plan en texto y su historial de runs. Si tiene un run lanzándose,
/// no (hay que esperar y detenerlo con Stop); los encolados se descartan.
#[tauri::command]
pub async fn delete_task(state: State<'_, TasksState>, id: String) -> Result<(), String> {
    check_id(&id)?;
    let inner = state.0.clone();
    let mut tasks = inner.tasks.lock().await;
    find_task(&tasks, &id)?;
    let mut runs = inner.runs.lock().await;
    if runs.runs.iter().any(|r| r.task_id == id && r.status == TaskRunStatus::Launching) {
        return Err("A run for this task is launching; wait for it to start and stop it first.".into());
    }
    let tasks_before = tasks.clone();
    tasks.tasks.retain(|t| t.id != id);
    persist_tasks(&inner, &tasks).await.inspect_err(|_| *tasks = tasks_before)?;
    runs.runs.retain(|r| r.task_id != id);
    if let Err(e) = persist_runs(&inner, &runs).await {
        eprintln!("tasks: {e}");
    }
    let dir = plan_dir(&inner, &id);
    blocking(move || match std::fs::remove_dir_all(&dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            eprintln!("tasks: couldn't remove {}: {e}", dir.display());
            Ok(())
        }
        _ => Ok(()),
    })
    .await
}

/// Tareas, más recientes primero. Con `repo_path`, solo las de ese repo.
#[tauri::command]
pub async fn list_tasks(state: State<'_, TasksState>, repo_path: Option<String>) -> Result<Vec<Task>, String> {
    let want = match repo_path.map(|p| p.trim().to_string()).filter(|p| !p.is_empty()) {
        // Se compara contra la ruta canonicalizada (así se guardan); si ya no existe,
        // contra el texto tal cual.
        Some(p) => Some(blocking(move || Ok(Path::new(&p).canonicalize().map(|c| c.to_string_lossy().into_owned()).unwrap_or(p))).await?),
        None => None,
    };
    let mut tasks: Vec<Task> = state
        .0
        .tasks
        .lock()
        .await
        .tasks
        .iter()
        .filter(|t| want.as_ref().is_none_or(|w| &t.repo_path == w))
        .cloned()
        .collect();
    tasks.sort_by_key(|t| std::cmp::Reverse(t.created_at));
    Ok(tasks)
}

#[tauri::command]
pub async fn set_task_done(state: State<'_, TasksState>, id: String, done: bool) -> Result<Task, String> {
    check_id(&id)?;
    let inner = state.0.clone();
    let mut data = inner.tasks.lock().await;
    let before = data.clone();
    let task = data.tasks.iter_mut().find(|t| t.id == id).ok_or("That task no longer exists.")?;
    task.status = if done { TaskStatus::Done } else { TaskStatus::Todo };
    task.done_at = done.then(store::now_ms);
    let task = task.clone();
    if let Err(e) = persist_tasks(&inner, &data).await {
        *data = before;
        return Err(e);
    }
    Ok(task)
}

/// Ruta del plan para leerlo o pasárselo al workflow. Un archivo se vuelve a validar
/// (puede haberse movido o reemplazado por un symlink desde que se creó la tarea).
fn resolve_plan_path(task: &Task, text_path: PathBuf) -> Result<PathBuf, String> {
    match &task.plan {
        PlanRef::Text => {
            if text_path.is_file() {
                Ok(text_path)
            } else {
                Err("The saved plan text is missing.".into())
            }
        }
        PlanRef::File { path } => {
            let repo = validate::repo_root(&task.repo_path)?;
            validate::plan_file(&repo, path)
        }
    }
}

/// Contenido del plan, como texto (el frontend lo muestra sin interpretar HTML).
#[tauri::command]
pub async fn read_task_plan(state: State<'_, TasksState>, id: String) -> Result<String, String> {
    check_id(&id)?;
    let inner = state.0.clone();
    let task = find_task(&*inner.tasks.lock().await, &id)?;
    let text_path = text_plan_path(&inner, &id);
    blocking(move || {
        let path = resolve_plan_path(&task, text_path)?;
        use std::io::Read;
        let file = std::fs::File::open(&path).map_err(|e| format!("Couldn't read the plan: {e}"))?;
        let mut bytes = Vec::new();
        file.take(validate::MAX_PLAN_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("Couldn't read the plan: {e}"))?;
        if bytes.len() as u64 > validate::MAX_PLAN_BYTES {
            return Err(format!("The plan is too large to show (max {} KB).", validate::MAX_PLAN_BYTES / 1024));
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    })
    .await
}

/// Encola un run de `plan-task` para la tarea y dispara una pasada de la cola.
#[tauri::command]
pub async fn launch_task_run(
    state: State<'_, TasksState>,
    id: String,
    finish: Option<String>,
) -> Result<TaskRun, String> {
    check_id(&id)?;
    let inner = state.0.clone();
    let task = find_task(&*inner.tasks.lock().await, &id)?;
    // Sin `finish` explícito manda el del repo en Settings; si tampoco hay, "pr".
    let finish = match finish {
        Some(f) => Some(f),
        None => {
            let (config_path, repo) = (inner.config_path.clone(), task.repo_path.clone());
            blocking(move || {
                Ok(config::load_from(&config_path)
                    .ok()
                    .and_then(|c| config::find_by_path(&c, &repo).and_then(|m| m.finish.clone())))
            })
            .await?
        }
    };
    let finish = validate::finish(finish.as_deref())?;
    let text_path = text_plan_path(&inner, &id);
    let (cwd, prompt) = {
        let task = task.clone();
        let finish = finish.clone();
        blocking(move || {
            let repo = validate::repo_root(&task.repo_path)?;
            let plan = resolve_plan_path(&task, text_path)?;
            let prompt = validate::prompt(&plan, &task.title, &finish)?;
            Ok((repo.to_string_lossy().into_owned(), prompt))
        })
        .await?
    };

    // Un solo run activo por tarea. Si `claude agents` falla, se decide con lo que hay.
    let live = runs::list_runs().await.ok();
    let entry = {
        // Orden tasks → runs (como delete_task): la tarea no puede borrarse en el medio.
        let tasks = inner.tasks.lock().await;
        find_task(&tasks, &id)?;
        let mut data = inner.runs.lock().await;
        let now = store::now_ms();
        let current = data.runs.iter().filter(|r| r.task_id == id).max_by_key(|r| r.queued_at);
        if let Some(cur) = current {
            if store::is_active(cur, live.as_deref(), now) {
                return Err(match cur.status {
                    TaskRunStatus::Queued => "This task is already queued.",
                    TaskRunStatus::Launching => "This task is launching.",
                    _ => "This task already has a run in progress.",
                }
                .into());
            }
        }
        let entry = TaskRun {
            task_id: id.clone(),
            prompt,
            finish,
            run_id: None,
            session_id: None,
            cwd,
            queued_at: now,
            launched_at: None,
            status: TaskRunStatus::Queued,
            error: None,
        };
        let before = data.clone();
        data.runs.push(entry.clone());
        store::prune(&mut data);
        if let Err(e) = persist_runs(&inner, &data).await {
            *data = before;
            return Err(e);
        }
        entry
    };

    // La pasada (que puede lanzar varios en serie) corre aparte: el click vuelve enseguida
    // con el run en cola y el frontend ve el cambio de estado en el próximo poll.
    let pumped = inner.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = pump(&pumped).await {
            eprintln!("tasks: {e}");
        }
    });
    Ok(entry)
}

/// Historial de runs de tareas (de una o de todas), más recientes primero.
#[tauri::command]
pub async fn list_task_runs(state: State<'_, TasksState>, task_id: Option<String>) -> Result<Vec<TaskRun>, String> {
    let mut runs: Vec<TaskRun> = state
        .0
        .runs
        .lock()
        .await
        .runs
        .iter()
        .filter(|r| task_id.as_ref().is_none_or(|t| &r.task_id == t))
        .cloned()
        .collect();
    runs.sort_by_key(|r| std::cmp::Reverse(r.queued_at));
    Ok(runs)
}

/// Saca de la cola los runs pendientes de la tarea. Para detener uno lanzado: `stop_run`.
#[tauri::command]
pub async fn cancel_task_run(state: State<'_, TasksState>, task_id: String) -> Result<(), String> {
    check_id(&task_id)?;
    let inner = state.0.clone();
    let mut data = inner.runs.lock().await;
    if data.runs.iter().any(|r| r.task_id == task_id && r.status == TaskRunStatus::Launching) {
        return Err("The run is already launching; wait for it to start and stop it instead.".into());
    }
    let before = data.clone();
    data.runs.retain(|r| !(r.task_id == task_id && r.status == TaskRunStatus::Queued));
    if data.runs.len() == before.runs.len() {
        return Err("This task has no queued runs.".into());
    }
    if let Err(e) = persist_runs(&inner, &data).await {
        *data = before;
        return Err(e);
    }
    Ok(())
}
