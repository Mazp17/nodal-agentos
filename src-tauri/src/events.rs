//! Aviso a la UI de que algo cambió en la base: evento `nodal://changed` con
//! `{kind, projectId?}`. La UI vuelve a pedir lo que muestra; el evento no lleva datos.
//!
//! Debounce simple: los avisos se juntan durante `DEBOUNCE` y se emiten sin repetir. Un
//! aviso sin proyecto cubre a los del mismo `kind` con proyecto.

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

/// Payload de `nodal://changed` (espejo de `ChangedEvent` en `api.ts`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Changed {
    pub kind: Kind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

/// Avisos acumulados entre emisiones.
#[derive(Debug, Default)]
struct Pending {
    items: BTreeSet<Changed>,
    scheduled: bool,
}

impl Pending {
    /// Agrega un aviso; `true` si hay que programar la emisión.
    fn push(&mut self, c: Changed) -> bool {
        self.items.insert(c);
        !std::mem::replace(&mut self.scheduled, true)
    }

    /// Lo que hay que emitir, sin los avisos con proyecto cubiertos por uno global.
    fn drain(&mut self) -> Vec<Changed> {
        self.scheduled = false;
        let items = std::mem::take(&mut self.items);
        let global: BTreeSet<Kind> = items.iter().filter(|c| c.project_id.is_none()).map(|c| c.kind).collect();
        items.into_iter().filter(|c| c.project_id.is_none() || !global.contains(&c.kind)).collect()
    }
}

/// Emisor con debounce. Sin `AppHandle` (tests) no hace nada.
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

    /// Varios `kind` para el mismo proyecto (o todos).
    pub fn notify_all(&self, kinds: &[Kind], project_id: Option<&str>) {
        for k in kinds {
            self.notify(*k, project_id);
        }
    }
}

/// Desde cualquier lugar con `AppHandle` (el emisor vive como estado de Tauri).
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
        assert!(p.push(c(Kind::Queue, None)), "tras drenar se vuelve a programar");
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
