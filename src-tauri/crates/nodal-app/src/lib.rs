//! Application/use-case layer: orchestrates the domain, the store and the ports
//! (implemented by nodal-host, nodal-linear and the shell) behind one `App` facade per
//! context. No tauri dependency.
#![forbid(unsafe_code)]

mod core;
mod flows;
#[cfg(test)]
mod testutil;

pub mod agent_api;
pub mod board;
pub mod chats;
pub mod execution;
pub mod sessions;
pub mod sources;
pub mod system;
