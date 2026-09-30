//! Linear in read-only mode. All calls go out from Rust, so the API key never goes through
//! the webview and the CSP does not need to open `connect-src` to linear.app.
//!
//! The client, model and error types live in `nodal_linear`. The key is read through the
//! sources hub's shared `Secrets` (provider `linear`, account `linear-api-key`), with Linear's
//! typed errors; it is never serialized to the frontend nor logged.

use std::sync::Arc;

use nodal_app::sources::SourcesHub;
use nodal_linear::{http_client, IssueDetail, LinearClient};
pub use nodal_linear::LinearError;
use tauri::State;

use crate::secrets::Secrets;

const PROVIDER: &str = "linear";

/// HTTP client shared with the sources hub's Linear provider (built once, before the hub, so
/// both sides reuse the same connection pool).
pub struct LinearState {
    http: nodal_linear::HttpClient,
}

impl LinearState {
    /// `secrets` isn't read from here anymore (`linear_issue_detail` goes through the hub's
    /// `Secrets` instead): kept as a parameter so the shell's construction order (built before
    /// the hub) doesn't need to change.
    pub fn new(_secrets: Secrets) -> Self {
        let ua = concat!("nodal/", env!("CARGO_PKG_VERSION"));
        Self { http: http_client(ua) }
    }

    /// For the hub's provider registry (built with the same client): same HTTP client.
    pub(crate) fn http(&self) -> &nodal_linear::HttpClient {
        &self.http
    }
}

/// Full detail of an issue for the side panel. `issue_id` accepts a UUID or an
/// identifier ("ACME-8").
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn linear_issue_detail(
    state: State<'_, LinearState>,
    hub: State<'_, Arc<SourcesHub>>,
    issue_id: String,
) -> Result<IssueDetail, LinearError> {
    let key = hub.secrets.get(PROVIDER).await.map_err(LinearError::Keychain)?.ok_or(LinearError::MissingKey)?;
    LinearClient::new(&state.http, &key).issue_detail(&issue_id).await
}
