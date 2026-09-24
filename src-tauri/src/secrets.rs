//! API keys de los proveedores en el llavero de macOS (crate `keyring`), con caché en memoria.
//! Servicio `io.github.mazp17.nodal`, cuenta `<provider>-api-key` (p. ej. `linear-api-key`).
//! Las keys nunca se serializan hacia el frontend ni se loguean.
//!
//! Hay una sola instancia (`Secrets::keychain()`, creada en `lib.rs`): se registra como
//! estado de Tauri (`State<Secrets>`, la usan los proveedores) y `linear::LinearState` guarda
//! un clon (misma caché) para los comandos `linear_*`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// Las llamadas al llavero son bloqueantes (y pueden esperar un diálogo del sistema).
use crate::util::blocking;

/// Debug builds use their own service so they don't read or overwrite the installed app's keys.
pub const SERVICE: &str = if crate::util::paths::DEV { "io.github.mazp17.nodal.dev" } else { "io.github.mazp17.nodal" };

/// Dónde se guardan de verdad las keys. En la app es el llavero; en tests, memoria.
pub trait SecretBackend: Send + Sync + 'static {
    fn read(&self, account: &str) -> Result<Option<String>, String>;
    fn write(&self, account: &str, value: &str) -> Result<(), String>;
    fn delete(&self, account: &str) -> Result<(), String>;
}

/// Llavero del sistema.
pub struct Keychain;

impl Keychain {
    fn entry(account: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(SERVICE, account).map_err(|e| format!("Could not open the keychain: {e}"))
    }
}

impl SecretBackend for Keychain {
    fn read(&self, account: &str) -> Result<Option<String>, String> {
        match Self::entry(account)?.get_password() {
            Ok(k) => Ok(Some(k)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("Could not read the keychain: {e}")),
        }
    }

    fn write(&self, account: &str, value: &str) -> Result<(), String> {
        Self::entry(account)?.set_password(value).map_err(|e| format!("Could not save to the keychain: {e}"))
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        match Self::entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("Could not delete from the keychain: {e}")),
        }
    }
}

/// `linear` → `linear-api-key`. El nombre del proveedor tiene que ser `[a-z0-9_]+`.
pub fn account_for(provider: &str) -> Result<String, String> {
    let ok = !provider.is_empty()
        && provider.len() <= 40
        && provider.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !ok {
        return Err(format!("Invalid provider name \"{provider}\"."));
    }
    Ok(format!("{provider}-api-key"))
}

/// Keys por proveedor. Cada proveedor se lee del backend solo la primera vez.
#[derive(Clone)]
pub struct Secrets {
    backend: Arc<dyn SecretBackend>,
    /// Ausente = todavía no se leyó; `Some(None)` = se leyó y no hay key.
    cache: Arc<Mutex<HashMap<String, Option<String>>>>,
}

impl Secrets {
    /// Llavero del sistema. Los clones comparten backend y caché.
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

    /// Key del proveedor (recortada; vacía cuenta como ausente).
    pub async fn get(&self, provider: &str) -> Result<Option<String>, String> {
        if let Some(v) = self.cached(provider) {
            return Ok(v);
        }
        let account = account_for(provider)?;
        let backend = self.backend.clone();
        let loaded = blocking(move || backend.read(&account)).await?;
        let loaded = loaded.map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
        // Si un set/delete terminó mientras leíamos, su valor gana sobre esta lectura.
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        Ok(cache.entry(provider.to_string()).or_insert(loaded).clone())
    }

    /// Guarda la key (recortada). Una key vacía equivale a borrarla.
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
pub mod testing {
    use super::*;

    /// Backend en memoria que cuenta las lecturas (para probar la caché).
    #[derive(Default, Clone)]
    pub struct MemoryBackend {
        pub data: Arc<Mutex<HashMap<String, String>>>,
        pub reads: Arc<Mutex<usize>>,
    }

    impl SecretBackend for MemoryBackend {
        fn read(&self, account: &str) -> Result<Option<String>, String> {
            *self.reads.lock().unwrap() += 1;
            Ok(self.data.lock().unwrap().get(account).cloned())
        }
        fn write(&self, account: &str, value: &str) -> Result<(), String> {
            self.data.lock().unwrap().insert(account.into(), value.into());
            Ok(())
        }
        fn delete(&self, account: &str) -> Result<(), String> {
            self.data.lock().unwrap().remove(account);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::MemoryBackend;
    use super::*;

    #[test]
    fn account_names() {
        assert_eq!(account_for("linear").unwrap(), "linear-api-key");
        assert_eq!(account_for("azure_devops").unwrap(), "azure_devops-api-key");
        assert!(account_for("").is_err());
        assert!(account_for("Linear").is_err());
        assert!(account_for("a/b").is_err());
    }

    #[test]
    fn get_set_delete_with_cache() {
        tauri::async_runtime::block_on(async {
            let mem = MemoryBackend::default();
            mem.data.lock().unwrap().insert("linear-api-key".into(), "  lin_abc \n".into());
            let s = Secrets::with_backend(mem.clone());

            assert_eq!(s.get("linear").await.unwrap().as_deref(), Some("lin_abc"));
            assert_eq!(s.get("linear").await.unwrap().as_deref(), Some("lin_abc"));
            assert_eq!(*mem.reads.lock().unwrap(), 1, "la segunda lectura sale de la caché");

            assert_eq!(s.get("asana").await.unwrap(), None);
            s.set("asana", " as_1 ").await.unwrap();
            assert_eq!(s.get("asana").await.unwrap().as_deref(), Some("as_1"));
            assert_eq!(mem.data.lock().unwrap().get("asana-api-key").map(String::as_str), Some("as_1"));

            s.delete("linear").await.unwrap();
            assert_eq!(s.get("linear").await.unwrap(), None);
            assert!(!mem.data.lock().unwrap().contains_key("linear-api-key"));

            s.set("asana", "   ").await.unwrap();
            assert_eq!(s.get("asana").await.unwrap(), None);
            assert!(s.get("Bad/Name").await.is_err());
        });
    }
}
