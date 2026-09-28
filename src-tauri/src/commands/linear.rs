//! Linear in read-only mode. All calls go out from Rust, so the API key never goes through
//! the webview and the CSP does not need to open `connect-src` to linear.app.
//!
//! The client, model and error types live in `nodal_linear`. The key is a facade over
//! `crate::secrets` (provider `linear`, account `linear-api-key`), with Linear's typed
//! errors; it is never serialized to the frontend nor logged.

use nodal_linear::{http_client, IssueDetail, LinearClient};
pub use nodal_linear::LinearError;
use tauri::State;

use crate::secrets::Secrets;

const PROVIDER: &str = "linear";

/// Clone of the `Secrets` registered as Tauri state (same cache).
struct KeyCache(Secrets);

impl KeyCache {
    fn new(secrets: Secrets) -> Self {
        Self(secrets)
    }

    /// Current key, reading the keychain only the first time.
    async fn load(&self) -> Result<Option<String>, LinearError> {
        self.0.get(PROVIDER).await.map_err(LinearError::Keychain)
    }

    async fn require(&self) -> Result<String, LinearError> {
        self.load().await?.ok_or(LinearError::MissingKey)
    }
}

pub struct LinearState {
    http: reqwest::Client,
    key: KeyCache,
}

impl LinearState {
    /// `secrets` is the shared instance (`State<Secrets>`): a key saved from `linear_*`
    /// or from `provider_*` is visible on both sides.
    pub fn new(secrets: Secrets) -> Self {
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
pub async fn linear_issue_detail(state: State<'_, LinearState>, issue_id: String) -> Result<IssueDetail, LinearError> {
    let key = state.key.require().await?;
    LinearClient::new(&state.http, &key).issue_detail(&issue_id).await
}
