//! Notifies the UI that something changed in the database: event `nodal://changed` with
//! `{kind, projectId?}`. The UI re-fetches what it shows; the event carries no data.
//!
//! Simple debounce: notices are collected for `DEBOUNCE` and emitted without duplicates. A
//! notice without a project covers those of the same `kind` with a project.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

pub const CHANGED: &str = "nodal://changed";
const DEBOUNCE: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Tasks,
    Runs,
    Queue,
    Sources,
    Projects,
}

/// Payload of `nodal://changed` (mirror of `ChangedEvent` in `api.ts`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Changed {
    pub kind: Kind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

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

/// From anywhere with an `AppHandle` (the emitter lives as Tauri state).
pub fn notify(app: &AppHandle, kinds: &[Kind], project_id: Option<&str>) {
    if let Some(ev) = app.try_state::<Events>() {
        ev.notify_all(kinds, project_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(kind: Kind, p: Option<&str>) -> Changed {
        Changed { kind, project_id: p.map(str::to_string) }
    }

    #[test]
    fn pending_schedules_once_and_dedupes() {
        let mut p = Pending::default();
        assert!(p.push(c(Kind::Tasks, Some("p1"))));
        assert!(!p.push(c(Kind::Tasks, Some("p1"))));
        assert!(!p.push(c(Kind::Runs, None)));
        assert_eq!(p.drain(), vec![c(Kind::Tasks, Some("p1")), c(Kind::Runs, None)]);
        assert!(p.push(c(Kind::Queue, None)), "after draining it schedules again");
    }

    #[test]
    fn global_notice_covers_project_ones() {
        let mut p = Pending::default();
        p.push(c(Kind::Tasks, Some("p1")));
        p.push(c(Kind::Tasks, None));
        p.push(c(Kind::Sources, Some("p2")));
        assert_eq!(p.drain(), vec![c(Kind::Tasks, None), c(Kind::Sources, Some("p2"))]);
    }

    #[test]
    fn payload_shape() {
        let v = serde_json::to_value(c(Kind::Sources, Some("p1"))).unwrap();
        assert_eq!(v, serde_json::json!({"kind": "sources", "projectId": "p1"}));
        let v = serde_json::to_value(c(Kind::Queue, None)).unwrap();
        assert_eq!(v, serde_json::json!({"kind": "queue"}));
    }

    #[test]
    fn events_without_app_are_noop() {
        Events::default().notify(Kind::Tasks, None);
    }
}
