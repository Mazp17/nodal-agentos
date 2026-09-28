//! Provider API keys in the macOS keychain (crate `keyring`). Service
//! `io.github.mazp17.nodal`, account `<provider>-api-key` (e.g. `linear-api-key`). Keys are
//! never serialized to the frontend or logged.

use nodal_domain::ports::SecretStore;

/// Debug builds use their own service so they don't read or overwrite the installed app's keys.
pub const SERVICE: &str = if nodal_domain::DEV { "io.github.mazp17.nodal.dev" } else { "io.github.mazp17.nodal" };

/// System keychain.
pub struct Keychain;

impl Keychain {
    fn entry(account: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(SERVICE, account).map_err(|e| format!("Could not open the keychain: {e}"))
    }
}

impl SecretStore for Keychain {
    fn read(&self, account: &str) -> Result<Option<String>, nodal_domain::error::HostError> {
        match Self::entry(account)?.get_password() {
            Ok(k) => Ok(Some(k)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("Could not read the keychain: {e}").into()),
        }
    }

    fn write(&self, account: &str, value: &str) -> Result<(), nodal_domain::error::HostError> {
        Self::entry(account)?.set_password(value).map_err(|e| format!("Could not save to the keychain: {e}"))?;
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), nodal_domain::error::HostError> {
        match Self::entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("Could not delete from the keychain: {e}").into()),
        }
    }
}
