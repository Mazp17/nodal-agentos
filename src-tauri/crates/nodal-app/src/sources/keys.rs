//! Provider API keys, with an in-memory cache in front of the `SecretStore` port. Moved from
//! the shell's `secrets.rs`; the keychain backend and the `keychain()` constructor stay
//! there (this crate doesn't depend on `nodal-host`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use nodal_domain::ports::SecretStore;

use crate::core::blocking;

/// `linear` → `linear-api-key`. The provider name must be `[a-z0-9_]+`.
pub fn account_for(provider: &str) -> Result<String, String> {
    let ok = !provider.is_empty()
        && provider.len() <= 40
        && provider
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !ok {
        return Err(format!("Invalid provider name \"{provider}\"."));
    }
    Ok(format!("{provider}-api-key"))
}

/// Keys per provider. Each provider is read from the backend only the first time.
#[derive(Clone)]
pub struct Secrets {
    store: Arc<dyn SecretStore>,
    /// Absent = not read yet; `Some(None)` = read and there is no key.
    cache: Arc<Mutex<HashMap<String, Option<String>>>>,
}

impl Secrets {
    pub fn new(store: Arc<dyn SecretStore>) -> Self {
        Self {
            store,
            cache: Arc::default(),
        }
    }

    fn cached(&self, provider: &str) -> Option<Option<String>> {
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(provider)
            .cloned()
    }

    fn set_cached(&self, provider: &str, v: Option<String>) {
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(provider.to_string(), v);
    }

    /// The provider's key (trimmed; empty counts as absent).
    pub async fn get(&self, provider: &str) -> Result<Option<String>, String> {
        if let Some(v) = self.cached(provider) {
            return Ok(v);
        }
        let account = account_for(provider)?;
        let store = self.store.clone();
        let loaded = blocking(move || store.read(&account).map_err(String::from)).await?;
        let loaded = loaded
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty());
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
        let store = self.store.clone();
        let k = key.clone();
        blocking(move || store.write(&account, &k).map_err(String::from)).await?;
        self.set_cached(provider, Some(key));
        Ok(())
    }

    pub async fn delete(&self, provider: &str) -> Result<(), String> {
        let account = account_for(provider)?;
        let store = self.store.clone();
        blocking(move || store.delete(&account).map_err(String::from)).await?;
        self.set_cached(provider, None);
        Ok(())
    }
}

#[cfg(test)]
pub mod testing {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use nodal_domain::error::HostError;
    use nodal_domain::ports::SecretStore;

    /// In-memory backend that counts reads (to test the cache).
    #[derive(Default, Clone)]
    pub struct MemoryBackend {
        pub data: Arc<Mutex<HashMap<String, String>>>,
        pub reads: Arc<Mutex<usize>>,
    }

    impl SecretStore for MemoryBackend {
        fn read(&self, account: &str) -> Result<Option<String>, HostError> {
            *self.reads.lock().unwrap() += 1;
            Ok(self.data.lock().unwrap().get(account).cloned())
        }
        fn write(&self, account: &str, value: &str) -> Result<(), HostError> {
            self.data
                .lock()
                .unwrap()
                .insert(account.into(), value.into());
            Ok(())
        }
        fn delete(&self, account: &str) -> Result<(), HostError> {
            self.data.lock().unwrap().remove(account);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests;
