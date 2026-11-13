//! Everything specific to the `claude` CLI: resolving and running it, its files on disk,
//! its config, its agent catalog and the `-p` stream-json protocol chats speak.

pub mod bin;
pub mod catalog;
pub mod chats;
pub mod cli;
pub mod fs;
pub mod session_location;
pub mod settings;
pub mod stream_json;
#[cfg(test)]
mod stream_json_live;
pub mod trust;
