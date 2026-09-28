//! Linear in read-only mode. All calls go out from Rust, so the API key never goes
//! through the webview and the CSP does not need to open `connect-src` to linear.app.

pub(crate) mod client;
mod detail;
mod error;
mod key;
pub(crate) mod model;

use client::{http_client, LinearClient};
use detail::IssueDetail;
pub use error::LinearError;
use key::KeyCache;
use tauri::State;

pub struct LinearState {
    http: reqwest::Client,
    key: KeyCache,
}

impl LinearState {
    /// `secrets` is the shared instance (`State<Secrets>`): a key saved from `linear_*`
    /// or from `provider_*` is visible on both sides.
    pub fn new(secrets: crate::secrets::Secrets) -> Self {
        Self { http: http_client(), key: KeyCache::new(secrets) }
    }

    /// For `providers::linear`: same HTTP client.
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

#[cfg(test)]
mod live_tests;
