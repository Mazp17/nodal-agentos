use serde::{Serialize, Serializer};
use std::fmt;

/// Error de cara a la UI. Se serializa como `{ kind, message }` para que el frontend
/// pueda elegir el estado vacío/de error correcto sin parsear textos.
/// Ningún mensaje incluye la API key.
#[derive(Debug, Clone, PartialEq)]
pub enum LinearError {
    MissingKey,
    InvalidKey,
    Network(String),
    RateLimited,
    Keychain(String),
    Api(String),
}

impl LinearError {
    pub fn kind(&self) -> &'static str {
        match self {
            LinearError::MissingKey => "missingKey",
            LinearError::InvalidKey => "invalidKey",
            LinearError::Network(_) => "network",
            LinearError::RateLimited => "rateLimited",
            LinearError::Keychain(_) => "keychain",
            LinearError::Api(_) => "api",
        }
    }
}

impl fmt::Display for LinearError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LinearError::MissingKey => {
                write!(f, "Falta la API key de Linear. Configurala en Ajustes.")
            }
            LinearError::InvalidKey => write!(
                f,
                "Linear rechazó la API key (inválida o revocada). Reemplazala en Ajustes."
            ),
            LinearError::Network(m) => write!(f, "{m}"),
            LinearError::RateLimited => write!(
                f,
                "Linear está limitando las peticiones (rate limit). Probá de nuevo en un rato."
            ),
            LinearError::Keychain(m) => write!(f, "{m}"),
            LinearError::Api(m) => write!(f, "Error de Linear: {m}"),
        }
    }
}

impl std::error::Error for LinearError {}

impl Serialize for LinearError {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("LinearError", 2)?;
        st.serialize_field("kind", self.kind())?;
        st.serialize_field("message", &self.to_string())?;
        st.end()
    }
}

impl From<reqwest::Error> for LinearError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_timeout() {
            LinearError::Network("Linear no respondió a tiempo. Revisá tu conexión.".into())
        } else if e.is_connect() {
            LinearError::Network(
                "No se pudo conectar con Linear. Revisá tu conexión a internet.".into(),
            )
        } else if e.is_decode() {
            LinearError::Api("respuesta ilegible".into())
        } else {
            // `without_url` por prolijidad; la key va en un header, nunca en la URL.
            LinearError::Network(format!("Fallo de red al hablar con Linear: {}", e.without_url()))
        }
    }
}
