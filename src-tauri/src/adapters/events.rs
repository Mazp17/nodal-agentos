//! Notifies the UI that something changed in the database: event `nodal://changed` with
//! `{kind, projectId?}`. The UI re-fetches what it shows; the event carries no data.
//!
//! Simple debounce: notices are collected for `DEBOUNCE` and emitted without duplicates. A
//! notice without a project covers those of the same `kind` with a project.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Emitter};

use nodal_domain::model::events::{Changed, ChangeKind as Kind, CHANGED};
use nodal_domain::ports::ChangeNotifier;

const DEBOUNCE: Duration = Duration::from_millis(150);

/// Notices accumulated between emissions.
#[derive(Debug, Default)]
struct Pending {
    items: BTreeSet<Changed>,
    scheduled: bool,
}

impl Pending {
    /// Adds a notice; `true` if the emission must be scheduled.
    fn push(&mut self, c: Changed) -> bool {
        self.items.insert(c);
        !std::mem::replace(&mut self.scheduled, true)
    }

    /// What must be emitted, minus the per-project notices covered by a global one.
    fn drain(&mut self) -> Vec<Changed> {
        self.scheduled = false;
        let items = std::mem::take(&mut self.items);
        let global: BTreeSet<Kind> = items.iter().filter(|c| c.project_id.is_none()).map(|c| c.kind).collect();
        items.into_iter().filter(|c| c.project_id.is_none() || !global.contains(&c.kind)).collect()
    }
}

/// Debounced emitter. Without an `AppHandle` (tests) it does nothing.
#[derive(Clone, Default)]
pub struct Events {
    app: Option<AppHandle>,
    pending: Arc<Mutex<Pending>>,
}

impl Events {
    pub fn new(app: AppHandle) -> Self {
        Self { app: Some(app), pending: Arc::default() }
    }

    pub fn notify(&self, kind: Kind, project_id: Option<&str>) {
        crate::telemetry::record_ipc_notify();
        let Some(app) = self.app.clone() else { return };
        let c = Changed { kind, project_id: project_id.map(str::to_string) };
        let schedule = match self.pending.lock() {
            Ok(mut p) => p.push(c),
            Err(_) => return,
        };
        if schedule {
            let pending = self.pending.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(DEBOUNCE).await;
                let items = pending.lock().map(|mut p| p.drain()).unwrap_or_default();
                for c in items {
                    if let Err(e) = app.emit(CHANGED, &c) {
                        eprintln!("events: {e}");
                    }
                }
            });
        }
    }

    /// Several `kind`s for the same project (or all of them).
    pub fn notify_all(&self, kinds: &[Kind], project_id: Option<&str>) {
        for k in kinds {
            self.notify(*k, project_id);
        }
    }
}

impl ChangeNotifier for Events {
    fn notify(&self, kind: Kind, project_id: Option<&str>) {
        Events::notify(self, kind, project_id)
    }
}

#[cfg(test)]
mod tests;
