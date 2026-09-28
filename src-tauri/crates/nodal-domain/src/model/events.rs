//! Payload of `nodal://changed`, the event that tells the UI something changed in the
//! database. The debounced emitter lives in the shell (`adapters::events`).

use serde::Serialize;

pub const CHANGED: &str = "nodal://changed";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Tasks,
    Runs,
    Queue,
    Sources,
    Projects,
    /// A project's chat list (created, renamed, used, deleted). What a chat streams goes
    /// through `nodal://chat` instead.
    Chats,
}

/// Payload of `nodal://changed` (mirror of `ChangedEvent` in `api.ts`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Changed {
    pub kind: ChangeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}
