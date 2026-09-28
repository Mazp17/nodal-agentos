//! Linear in read-only mode. All calls go out from Rust, so the API key never goes
//! through the webview and the CSP does not need to open `connect-src` to linear.app.
//!
//! The client, model and error types moved to `nodal_linear`; re-exported here so
//! current uses don't break.

mod key;

use key::KeyCache;
use nodal_linear::{http_client, IssueDetail, LinearClient};
pub use nodal_linear::LinearError;
use tauri::State;

pub struct LinearState {
    http: reqwest::Client,
    key: KeyCache,
}

impl LinearState {
    /// `secrets` is the shared instance (`State<Secrets>`): a key saved from `linear_*`
    /// or from `provider_*` is visible on both sides.
    pub fn new(secrets: crate::secrets::Secrets) -> Self {
        let ua = concat!("nodal/", env!("CARGO_PKG_VERSION"));
        Self { http: http_client(ua), key: KeyCache::new(secrets) }
    }

    /// For `providers::resolve_with_key`/`provider_set_key`: same HTTP client.
    pub(crate) fn http(&self) -> &reqwest::Client {
        &self.http
    }
}

/// Full detail of an issue for the side panel. `issue_id` accepts a UUID or an
/// identifier ("ACME-8").
#[tauri::command]
pub async fn linear_issue_detail(
    state: State<'_, LinearState>,
    issue_id: String,
) -> Result<IssueDetail, LinearError> {
    let key = state.key.require().await?;
    LinearClient::new(&state.http, &key).issue_detail(&issue_id).await
}
