//! API key de Linear en el llavero de macOS (crate `keyring`), con caché en memoria.
//! La key nunca se serializa hacia el frontend ni se loguea.

use super::error::LinearError;
use std::sync::Mutex;

// Misma convención que `secrets.rs` (F1-A): servicio `com.nodal.app`, cuenta
// `<provider>-api-key`. TODO(F1 integración): unificar con `secrets.rs` y borrar esto.
const SERVICE: &str = "com.nodal.app";
const ACCOUNT: &str = "linear-api-key";

fn entry() -> Result<keyring::Entry, LinearError> {
    keyring::Entry::new(SERVICE, ACCOUNT)
        .map_err(|e| LinearError::Keychain(format!("Could not open the keychain: {e}")))
}

fn read_keychain() -> Result<Option<String>, LinearError> {
    match entry()?.get_password() {
        Ok(k) if !k.trim().is_empty() => Ok(Some(k.trim().to_string())),
        Ok(_) | Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(LinearError::Keychain(format!("Could not read the keychain: {e}"))),
    }
}

fn write_keychain(key: &str) -> Result<(), LinearError> {
    entry()?
        .set_password(key)
        .map_err(|e| LinearError::Keychain(format!("Could not save to the keychain: {e}")))
}

fn delete_keychain() -> Result<(), LinearError> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(LinearError::Keychain(format!("Could not delete from the keychain: {e}"))),
    }
}

/// Las llamadas al llavero son bloqueantes (y pueden esperar un diálogo del sistema),
/// así que corren fuera del runtime async.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, LinearError> + Send + 'static,
) -> Result<T, LinearError> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| LinearError::Keychain(format!("Internal error reading the keychain: {e}")))?
}

/// `None` = todavía no se leyó el llavero; `Some(None)` = se leyó y no hay key.
#[derive(Default)]
pub struct KeyCache(Mutex<Option<Option<String>>>);

impl KeyCache {
    fn get_cached(&self) -> Option<Option<String>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn set_cached(&self, v: Option<String>) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(v);
    }

    /// Key actual, leyendo el llavero sólo la primera vez.
    pub async fn load(&self) -> Result<Option<String>, LinearError> {
        if let Some(cached) = self.get_cached() {
            return Ok(cached);
        }
        let loaded = blocking(read_keychain).await?;
        // Si un store/clear terminó mientras leíamos, su valor gana sobre esta lectura.
        let mut guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        Ok(guard.get_or_insert(loaded).clone())
    }

    pub async fn require(&self) -> Result<String, LinearError> {
        self.load().await?.ok_or(LinearError::MissingKey)
    }

    pub async fn store(&self, key: String) -> Result<(), LinearError> {
        let k = key.clone();
        blocking(move || write_keychain(&k)).await?;
        self.set_cached(Some(key));
        Ok(())
    }

    pub async fn clear(&self) -> Result<(), LinearError> {
        blocking(delete_keychain).await?;
        self.set_cached(None);
        Ok(())
    }
}
