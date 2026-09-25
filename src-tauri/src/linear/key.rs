//! Linear API key: facade over `crate::secrets` (provider `linear`, account
//! `linear-api-key` in the `io.github.mazp17.nodal` service), with Linear's typed errors.
//! The key is never serialized to the frontend nor logged.

use super::error::LinearError;
use crate::secrets::Secrets;

const PROVIDER: &str = "linear";

/// Clone of the `Secrets` registered as Tauri state (same cache).
pub struct KeyCache(Secrets);

impl KeyCache {
    pub fn new(secrets: Secrets) -> Self {
        Self(secrets)
    }

    /// Current key, reading the keychain only the first time.
    pub async fn load(&self) -> Result<Option<String>, LinearError> {
        self.0.get(PROVIDER).await.map_err(LinearError::Keychain)
    }

    pub async fn require(&self) -> Result<String, LinearError> {
        self.load().await?.ok_or(LinearError::MissingKey)
    }

    pub async fn store(&self, key: String) -> Result<(), LinearError> {
        self.0.set(PROVIDER, &key).await.map_err(LinearError::Keychain)
    }

    pub async fn clear(&self) -> Result<(), LinearError> {
        self.0.delete(PROVIDER).await.map_err(LinearError::Keychain)
    }
}
