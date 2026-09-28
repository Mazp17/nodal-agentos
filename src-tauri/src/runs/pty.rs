//! Moved to `nodal_host::pty`; re-exported so current uses don't break. The `Channel`/
//! `Webview` glue (this crate has tauri; the host crate doesn't) stays here, wrapped into
//! the plain callbacks `PtySessions::attach` takes.

use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{State, Webview};

pub use nodal_host::pty::PtySessions;

use super::terminal::is_valid_run_id;

/// Attaches to a background session: opens a pty of `cols`×`rows`, spawns `claude attach
/// <run_id>` into it and streams its output through `on_data` until it exits (reported once on
/// `on_exit`). The initial size must be right (it can't be corrected by an immediate resize:
/// verified in the spike, the TUI draws its first frame at whatever size the pty had at spawn).
// This many arguments is what the frontend calls with; it's a `tauri::command`, not a free
// choice, so each one is a separate JS argument.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn pty_attach(
    webview: Webview,
    state: State<'_, PtySessions>,
    run_id: String,
    cwd: Option<String>,
    cols: u16,
    rows: u16,
    on_data: Channel<InvokeResponseBody>,
    on_exit: Channel<Option<i32>>,
) -> Result<u32, String> {
    if !is_valid_run_id(&run_id) {
        return Err(format!("Invalid run id: \"{run_id}\"."));
    }
    let sessions = state.inner().clone();
    let owner = webview.label().to_string();
    // Captured now, before the (possibly slow) attach below, so a reload landing during it is
    // caught by `insert_current` instead of racing `close_owned_by`.
    let generation = sessions.generation_for(&owner);
    let on_data_cb: nodal_host::pty::OnData = Box::new(move |chunk| {
        let _ = on_data.send(InvokeResponseBody::Raw(chunk));
    });
    let on_exit_cb: nodal_host::pty::OnExit = Box::new(move |code| {
        let _ = on_exit.send(code);
    });
    tauri::async_runtime::spawn_blocking(move || sessions.attach(owner, generation, run_id, cwd, cols, rows, on_data_cb, on_exit_cb))
        .await
        .map_err(|e| format!("Internal error starting the terminal: {e}"))?
}

/// Hands `data` to the session's writer thread; never blocks on the actual write.
#[tauri::command]
pub async fn pty_write(state: State<'_, PtySessions>, session: u32, data: Vec<u8>) -> Result<(), String> {
    state.write(session, data)
}

/// Resizes the pty; `claude attach`'s TUI redraws on `SIGWINCH`, which this triggers.
#[tauri::command]
pub async fn pty_resize(state: State<'_, PtySessions>, session: u32, cols: u16, rows: u16) -> Result<(), String> {
    state.resize(session, cols, rows)
}

/// Detaches a session (idempotent: `Ok` even if `session` is unknown, e.g. it already exited).
/// This only detaches `claude attach`; the background session survives (see the module docs).
#[tauri::command]
pub async fn pty_close(state: State<'_, PtySessions>, session: u32) -> Result<(), String> {
    state.close(session);
    Ok(())
}
