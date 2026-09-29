//! The error type ports return: text meant for the UI (a display-ready string), wrapped so
//! it can be converted with `?` and printed like the strings the old code used directly.

/// Display-ready error message from a port. `Debug`/`Clone`/`Eq` so it can be asserted on
/// in tests and compared like the `String` errors it replaces.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct HostError(pub String);

impl From<String> for HostError {
    fn from(s: String) -> Self {
        HostError(s)
    }
}

impl From<&str> for HostError {
    fn from(s: &str) -> Self {
        HostError(s.to_string())
    }
}

impl From<HostError> for String {
    fn from(e: HostError) -> Self {
        e.0
    }
}
