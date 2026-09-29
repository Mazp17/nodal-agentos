//! Pure domain types and logic: no I/O, no tauri. See `ports.rs` (added by the
//! integrator) for the traits nodal-app uses to reach the outside world.
#![forbid(unsafe_code)]

/// Debug builds use their own app identifier, data dir and keychain service so they don't
/// collide with the installed app's.
pub const DEV: bool = cfg!(debug_assertions);

pub mod diff;
pub mod error;
pub mod ports;
pub mod serde_util;
#[cfg(any(test, feature = "test-support"))]
pub mod testutil;
pub mod util;

pub mod board;
pub mod execution;
pub mod model;
pub mod sessions;
pub mod sources;
