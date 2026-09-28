//! Generic model of a task provider's items (Linear issue, Asana task, ADO work item) and
//! errors. The provider trait itself is a port (`ports::TaskProvider`); adapters live in
//! nodal-linear.

use serde::Serialize;

use crate::model::{ExtKind, ExtProject, ExternalState, Priority, ScopeRef};

/// Provider error class: decides what the outbox does with the row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Network, 5xx: retry with backoff (up to `sync::MAX_ATTEMPTS`).
    Transient,
    /// Retry later and stop draining this provider for this pass.
    RateLimited,
    /// Missing or invalid key: same as `RateLimited`.
    Auth,
    /// The provider rejected the write (deleted item, no permission, invalid input): retrying
    /// makes no sense.
    Permanent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderError {
    pub kind: ErrorKind,
    pub message: String,
}

impl ProviderError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<ProviderError> for String {
    fn from(e: ProviderError) -> Self {
        e.message
    }
}

pub type ProviderResult<T> = Result<T, ProviderError>;

/// "Open" state kinds: the default for the importable listing and for auto-import.
pub const OPEN_KINDS: &[ExtKind] = &[
    ExtKind::Triage,
    ExtKind::Backlog,
    ExtKind::Unstarted,
    ExtKind::Started,
];

/// Lightweight reference to another item (the parent).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemRef {
    pub external_id: String,
    pub identifier: String,
    pub title: String,
    pub url: String,
}

/// Sub-item (sub-issue), with its state to mark the ones already done.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildItem {
    pub external_id: String,
    pub identifier: String,
    pub title: String,
    pub url: String,
    pub state: ExternalState,
}

/// A provider item (Linear issue, Asana task, ADO work item).
/// In the importable listing `description_md`, `parent`, `children` and `assignee` may
/// be empty: importing fetches the full item again with `pull`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalItem {
    /// Stable provider id (UUID in Linear). Unique per provider.
    pub external_id: String,
    /// Human-readable id (`ENG-142`).
    pub identifier: String,
    pub url: String,
    pub title: String,
    pub description_md: Option<String>,
    pub state: ExternalState,
    /// Scopes it belongs to (team and, if any, project).
    pub scopes: Vec<ScopeRef>,
    pub parent: Option<ItemRef>,
    pub children: Vec<ChildItem>,
    pub labels: Vec<String>,
    /// Assignee's display name in the provider (informational).
    pub assignee: Option<String>,
    pub priority: Priority,
    /// ISO 8601, as given by the provider.
    pub updated_at: String,
    /// ISO 8601. `None` if the provider doesn't report it.
    pub created_at: Option<String>,
    /// ISO 8601: when it was completed or canceled (`None` if open or unknown).
    pub closed_at: Option<String>,
}

impl ExternalItem {
    /// Provider project it belongs to (`project` scope), if any.
    pub fn project(&self) -> Option<ExtProject> {
        self.scopes
            .iter()
            .find(|s| s.kind == "project")
            .map(|s| ExtProject {
                id: s.id.clone(),
                name: s.name.clone(),
            })
    }
}

/// Query for the importable listing.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportQuery {
    pub scope: ScopeRef,
    /// Free text: title or identifier.
    pub text: Option<String>,
    /// Empty = all kinds.
    pub state_kinds: Vec<ExtKind>,
    /// Only items created after this instant (epoch ms). Used by auto-import.
    pub created_after: Option<i64>,
    /// Only items from this provider project (within `scope`): project rules.
    pub project_id: Option<String>,
    /// Besides `state_kinds`, the ones completed or canceled less than this many days ago
    /// (backfill of a project rule).
    pub closed_within_days: Option<u32>,
    pub cursor: Option<String>,
}

impl ImportQuery {
    pub fn new(scope: ScopeRef) -> Self {
        ImportQuery {
            scope,
            text: None,
            state_kinds: OPEN_KINDS.to_vec(),
            created_after: None,
            project_id: None,
            closed_within_days: None,
            cursor: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Page {
    pub items: Vec<ExternalItem>,
    pub next_cursor: Option<String>,
}
