//! Everything specific to the `claude` CLI: resolving and running it, its files on disk,
//! its config, its workflow/agent catalog and the `-p` stream-json protocol chats speak.

pub mod activity;
pub mod activity_files;
pub mod bin;
pub mod catalog;
pub mod chats;
pub mod cli;
pub mod fs;
pub mod live;
pub mod settings;
pub mod stream_json;
#[cfg(test)]
mod stream_json_live;
pub mod trust;
pub mod workflows;
