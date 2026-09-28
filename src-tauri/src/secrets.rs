//! Provider API keys in the macOS keychain (crate `keyring`), with an in-memory cache.
//! Service `io.github.mazp17.nodal`, account `<provider>-api-key` (e.g. `linear-api-key`).
//! Keys are never serialized to the frontend or logged.
//!
//! `Secrets` (the cache) and `account_for` moved to `nodal_app::sources::keys`, adapted to
//! the `SecretStore` port; `keychain()` below is the equivalent of the old
//! `Secrets::keychain()`, kept here because this crate is the one that knows about
//! `nodal_host::keychain::Keychain` (nodal-app doesn't depend on nodal-host).
//!
//! There is a single instance (`secrets::keychain()`, created in `lib.rs`): it is registered
//! as Tauri state (`State<Secrets>`, used by the providers) and `linear::LinearState` keeps a
//! clone (same cache) for the `linear_*` commands.

use std::sync::Arc;

/// Moved to `nodal_host::keychain`; re-exported so current uses don't break.
pub use nodal_host::keychain::Keychain;
/// Only the bridge is left; nothing in this crate calls it directly anymore.
#[allow(unused_imports)]
pub use nodal_host::keychain::SERVICE;

pub use nodal_app::sources::keys::Secrets;
/// Only the bridge is left; nothing in this crate calls it directly anymore.
#[allow(unused_imports)]
pub use nodal_app::sources::keys::account_for;

/// System keychain. Clones share the backend and cache. Equivalent to the old
/// `Secrets::keychain()`.
pub fn keychain() -> Secrets {
    Secrets::new(Arc::new(Keychain))
}
