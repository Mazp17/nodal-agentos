//! Adapters to the outside world (Claude Code CLI/files, git, keychain, pty, terminal)
//! implementing the ports declared in `nodal_domain::ports`. No tauri dependency: the
//! shell wires these into Tauri state and events.

mod adapters;
mod keychain;
mod paths;
mod plans;
mod pty;
mod repo;
mod terminal;
#[cfg(any(test, feature = "test-support"))]
pub mod testutil;

pub mod claude;
pub mod git;
