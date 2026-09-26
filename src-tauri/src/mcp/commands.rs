//! Tauri commands behind Settings → Diagnostics: see, stop and restart the MCP socket.

use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::State;

use super::server::{self, Listening};
use crate::work::Inner;

const NO_DB: &str = "Nodal's database didn't open, so there is nothing to serve.";

/// Owns the listener. Managed even without a database, so the UI can show why it is off.
#[derive(Default)]
pub struct McpState(Mutex<Slot>);

#[derive(Default)]
struct Slot {
    inner: Option<Arc<Inner>>,
    listening: Option<Listening>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStatus {
    /// `false` without a database: there is nothing to serve, so it can't start.
    available: bool,
    running: bool,
    socket: Option<String>,
    error: Option<String>,
}

impl McpState {
    /// Called once at startup; the error, if any, is kept for the status.
    pub fn start(&self, inner: Arc<Inner>) {
        let mut slot = self.lock();
        slot.inner = Some(inner);
        slot.restart();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Slot> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }
}

impl Slot {
    fn stop(&mut self) {
        if let Some(l) = self.listening.take() {
            l.stop();
        }
        self.error = None;
    }

    fn restart(&mut self) {
        self.stop();
        let Some(inner) = self.inner.clone() else {
            return;
        };
        match server::start(inner) {
            Ok(l) => self.listening = Some(l),
            Err(e) => {
                eprintln!("mcp: {e}");
                self.error = Some(e);
            }
        }
    }

    fn status(&self) -> McpStatus {
        McpStatus {
            available: self.inner.is_some(),
            running: self.listening.is_some(),
            socket: self.listening.as_ref().map(|l| l.path().display().to_string()),
            error: self.error.clone().or_else(|| self.inner.is_none().then(|| NO_DB.to_string())),
        }
    }
}

#[tauri::command]
pub fn mcp_status(state: State<'_, McpState>) -> McpStatus {
    state.lock().status()
}

/// Starts the server, or stops and starts it again if it was running.
#[tauri::command]
pub fn mcp_restart(state: State<'_, McpState>) -> McpStatus {
    let mut slot = state.lock();
    slot.restart();
    slot.status()
}

#[tauri::command]
pub fn mcp_stop(state: State<'_, McpState>) -> McpStatus {
    let mut slot = state.lock();
    slot.stop();
    slot.status()
}
