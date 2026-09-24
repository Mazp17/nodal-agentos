//! Trabajo: proyectos, repos y tareas (CRUD), cola única de runs, ejecutores,
//! worktrees, revisión automática y diff.
//!
//! - `ops`: CRUD síncrono sobre la base (los comandos lo corren con `with_db`);
//! - `launch`: armado de prompts y encolado de runs;
//! - `queue` y `transitions`: lógica pura de la cola y de los estados;
//! - `pump`: la pasada periódica que detecta fines, aplica transiciones y lanza;
//! - `commands`: los comandos de Tauri.

pub mod commands;
pub mod diff;
pub mod dto;
pub mod executors;
pub mod launch;
pub mod ops;
pub mod pump;
pub mod queue;
pub mod report;
pub mod transitions;
pub mod validate;
pub mod worktree;

#[cfg(test)]
pub(crate) mod testutil;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Manager};

use crate::db::Db;

const TICK: Duration = Duration::from_secs(5);

/// Rutas de las que depende el trabajo (inyectables en los tests).
#[derive(Debug, Clone)]
pub struct Env {
    /// `app_data_dir`: planes en `tasks/<id>/plan.md`, patches de runs detenidos.
    pub data_dir: PathBuf,
    /// `~/.nodal/worktrees`.
    pub worktrees_root: PathBuf,
    /// `~/.claude` (o `$CLAUDE_CONFIG_DIR`): agentes, workflows y plugins.
    pub claude_dir: Option<PathBuf>,
}

impl Env {
    pub fn plan_dir(&self, task_id: &str) -> PathBuf {
        self.data_dir.join("tasks").join(task_id)
    }
    pub fn text_plan_path(&self, task_id: &str) -> PathBuf {
        self.plan_dir(task_id).join("plan.md")
    }
    pub fn stopped_patch_path(&self, run_id: &str) -> PathBuf {
        self.data_dir.join("runs").join(run_id).join("stopped.patch")
    }
}

pub struct Inner {
    pub db: Db,
    pub env: Env,
    /// Serializa las pasadas de la cola: una sola a la vez decide y lanza.
    pub pump: tokio::sync::Mutex<()>,
}

#[derive(Clone)]
pub struct WorkState(pub Arc<Inner>);

/// Registra el estado y arranca la cola.
pub fn init(app: &AppHandle, db: Db) -> Result<(), String> {
    let data_dir = app.path().app_data_dir().map_err(|e| format!("Couldn't find the app data folder: {e}"))?;
    let env = Env {
        data_dir,
        worktrees_root: crate::util::paths::nodal_home()?.join("worktrees"),
        claude_dir: crate::runs::claude_fs::claude_config_dir(),
    };
    // Un `launching` de una sesión anterior no se sabe si llegó a lanzarse.
    if let Ok(conn) = db.lock() {
        if let Err(e) = crate::db::queries::runs::fail_interrupted_launches(&conn, crate::util::now_ms()) {
            eprintln!("work: {e}");
        }
    }
    let inner = Arc::new(Inner { db, env, pump: tokio::sync::Mutex::new(()) });
    app.manage(WorkState(inner.clone()));
    tauri::async_runtime::spawn(async move {
        loop {
            if let Err(e) = pump::pump(&inner).await {
                eprintln!("work: {e}");
            }
            tokio::time::sleep(TICK).await;
        }
    });
    Ok(())
}

/// Dispara una pasada de la cola en segundo plano (tras encolar o cancelar).
pub fn kick(inner: &Arc<Inner>) {
    let inner = inner.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = pump::pump(&inner).await {
            eprintln!("work: {e}");
        }
    });
}
