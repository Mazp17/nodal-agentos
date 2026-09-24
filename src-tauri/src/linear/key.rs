//! API key de Linear: fachada sobre `crate::secrets` (proveedor `linear`, cuenta
//! `linear-api-key` en el servicio `io.github.mazp17.nodal`), con los errores tipados de Linear.
//! La key nunca se serializa hacia el frontend ni se loguea.

use super::error::LinearError;
use crate::secrets::Secrets;

const PROVIDER: &str = "linear";

/// Clon del `Secrets` que se registra como estado de Tauri (misma caché).
pub struct KeyCache(Secrets);

impl KeyCache {
    pub fn new(secrets: Secrets) -> Self {
        Self(secrets)
    }

    /// Key actual, leyendo el llavero sólo la primera vez.
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
