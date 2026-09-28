//! Provider API keys in the macOS keychain (crate `keyring`), with an in-memory cache.
//! Service `io.github.mazp17.nodal`, account `<provider>-api-key` (e.g. `linear-api-key`).
//! Keys are never serialized to the frontend or logged.
//!
//! There is a single instance (`Secrets::keychain()`, created in `lib.rs`): it is registered as
//! Tauri state (`State<Secrets>`, used by the providers) and `linear::LinearState` keeps a
//! clone (same cache) for the `linear_*` commands.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// Keychain calls are blocking (and may wait on a system dialog).
use crate::util::blocking;

/// Moved to `nodal_host::keychain`; re-exported so current uses don't break.
pub use nodal_host::keychain::Keychain;
/// Only the bridge is left; nothing in this crate calls it directly anymore.
#[allow(unused_imports)]
pub use nodal_host::keychain::SERVICE;

/// Where the keys are actually stored. In the app it's the keychain; in tests, memory.
pub trait SecretBackend: Send + Sync + 'static {
    fn read(&self, account: &str) -> Result<Option<String>, String>;
    fn write(&self, account: &str, value: &str) -> Result<(), String>;
    fn delete(&self, account: &str) -> Result<(), String>;
}

impl SecretBackend for Keychain {
    fn read(&self, account: &str) -> Result<Option<String>, String> {
        nodal_domain::ports::SecretStore::read(self, account).map_err(String::from)
    }

    fn write(&self, account: &str, value: &str) -> Result<(), String> {
        nodal_domain::ports::SecretStore::write(self, account, value).map_err(String::from)
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        nodal_domain::ports::SecretStore::delete(self, account).map_err(String::from)
    }
}

/// `linear` → `linear-api-key`. The provider name must be `[a-z0-9_]+`.
pub fn account_for(provider: &str) -> Result<String, String> {
    let ok = !provider.is_empty()
        && provider.len() <= 40
        && provider.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !ok {
        return Err(format!("Invalid provider name \"{provider}\"."));
    }
    Ok(format!("{provider}-api-key"))
}

/// Keys per provider. Each provider is read from the backend only the first time.
#[derive(Clone)]
pub struct Secrets {
    backend: Arc<dyn SecretBackend>,
    /// Absent = not read yet; `Some(None)` = read and there is no key.
    cache: Arc<Mutex<HashMap<String, Option<String>>>>,
}

impl Secrets {
    /// System keychain. Clones share the backend and cache.
    pub fn keychain() -> Self {
        Self::with_backend(Keychain)
    }

    pub fn with_backend(backend: impl SecretBackend) -> Self {
        Self { backend: Arc::new(backend), cache: Arc::default() }
    }

    fn cached(&self, provider: &str) -> Option<Option<String>> {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).get(provider).cloned()
    }

    fn set_cached(&self, provider: &str, v: Option<String>) {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).insert(provider.to_string(), v);
    }

    /// The provider's key (trimmed; empty counts as absent).
    pub async fn get(&self, provider: &str) -> Result<Option<String>, String> {
        if let Some(v) = self.cached(provider) {
            return Ok(v);
        }
        let account = account_for(provider)?;
        let backend = self.backend.clone();
        let loaded = blocking(move || backend.read(&account)).await?;
        let loaded = loaded.map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
        // If a set/delete finished while we were reading, its value wins over this read.
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        Ok(cache.entry(provider.to_string()).or_insert(loaded).clone())
    }

    /// Saves the key (trimmed). An empty key is the same as deleting it.
    pub async fn set(&self, provider: &str, key: &str) -> Result<(), String> {
        let key = key.trim().to_string();
        if key.is_empty() {
            return self.delete(provider).await;
        }
        let account = account_for(provider)?;
        let backend = self.backend.clone();
        let k = key.clone();
        blocking(move || backend.write(&account, &k)).await?;
        self.set_cached(provider, Some(key));
        Ok(())
    }

    pub async fn delete(&self, provider: &str) -> Result<(), String> {
        let account = account_for(provider)?;
        let backend = self.backend.clone();
        blocking(move || backend.delete(&account)).await?;
        self.set_cached(provider, None);
        Ok(())
    }
}

#[cfg(test)]
pub mod testing;

#[cfg(test)]
mod tests;
