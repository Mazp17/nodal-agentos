//! Machinery moved to `nodal_host::claude::chats::ChatProcesses`; DB side effects moved to
//! `chats::hooks::LegacyHooks`. This file is now a thin bridge so `Chats::new(db, events,
//! emit)` and its old call sites keep working.

#[cfg(test)]
use std::path::PathBuf;
use std::sync::Arc;
#[cfg(test)]
use std::sync::Mutex;
#[cfg(test)]
use std::time::{Duration, Instant};

use nodal_domain::ports::{ChatHooks, ChatRuntime, ChatSink};

pub use nodal_host::claude::chats::ChatProcesses;
/// Only the bridge is left; nothing in this crate calls them directly anymore.
#[allow(unused_imports)]
pub use nodal_host::claude::chats::{DENY_MESSAGE, IDLE_TIMEOUT};

use crate::db::Db;
use crate::events::Events;
use crate::runs::claude_fs;

use super::hooks::LegacyHooks;

/// Moved to `nodal_domain::model::chat`; re-exported so current uses don't break.
pub use nodal_domain::model::chat::CHAT_EVENT as EVENT;
pub use nodal_domain::model::chat::{ChatEnvelope, ChatLive, ChatSpec as Spec};
/// Only the bridge is left in non-test builds (used by `process/tests.rs`).
#[allow(unused_imports)]
pub use nodal_domain::model::chat::{ChatEvent, HostEvent, PermissionRequest, RunState, StreamEvent};

pub type Emit = Arc<dyn Fn(ChatEnvelope) + Send + Sync>;

struct EmitSink(Emit);

impl ChatSink for EmitSink {
    fn emit(&self, ev: ChatEnvelope) {
        (self.0)(ev)
    }
}

fn runtime_handle() -> tokio::runtime::Handle {
    tauri::async_runtime::handle().inner().clone()
}

/// The chat processes. Cheap to clone.
#[derive(Clone)]
pub struct Chats(pub Arc<ChatProcesses>);

impl Chats {
    pub fn new(db: Db, events: Events, emit: Emit) -> Self {
        let hooks: Arc<dyn ChatHooks> = Arc::new(LegacyHooks { db, events, claude_dir: claude_fs::claude_config_dir() });
        let sink: Arc<dyn ChatSink> = Arc::new(EmitSink(emit));
        Chats(ChatProcesses::new(sink, hooks, runtime_handle()))
    }

    /// Test-only: points the process at a fake `claude` script instead of the real binary.
    #[cfg(test)]
    fn with_program(db: Db, events: Events, emit: Emit, program: Option<PathBuf>, claude_dir: Option<PathBuf>) -> Self {
        let hooks: Arc<dyn ChatHooks> = Arc::new(LegacyHooks { db, events, claude_dir });
        let sink: Arc<dyn ChatSink> = Arc::new(EmitSink(emit));
        Chats(ChatProcesses::with_program(sink, hooks, runtime_handle(), program))
    }

    pub fn send(&self, chat_id: &str, project_id: &str, session_id: Option<&str>, spec: Spec, text: &str) -> Result<(), String> {
        Ok(self.0.send(chat_id, project_id, session_id, spec, text)?)
    }

    pub fn respond(&self, chat_id: &str, request_id: &str, allow: bool, message: Option<&str>) -> Result<(), String> {
        Ok(self.0.respond(chat_id, request_id, allow, message)?)
    }

    pub fn interrupt(&self, chat_id: &str) -> Result<(), String> {
        Ok(self.0.interrupt(chat_id)?)
    }

    pub fn stop(&self, chat_id: &str) {
        self.0.stop(chat_id)
    }

    pub fn stop_project(&self, project_id: &str) {
        self.0.stop_project(project_id)
    }

    pub fn live(&self, chat_id: &str) -> ChatLive {
        self.0.live(chat_id)
    }

    pub fn start_reaper(&self) {
        self.0.start_reaper()
    }

    #[cfg(test)]
    fn reap_idle(&self, max_idle: Duration, now: Instant) -> usize {
        self.0.reap_idle(max_idle, now)
    }

    /// Drops every running process without a clean detach (test cleanup only: the fake
    /// `claude` scripts are killed with the temp dir anyway).
    #[cfg(test)]
    fn clear(&self) {
        self.0.clear();
    }
}

#[cfg(test)]
mod tests;
