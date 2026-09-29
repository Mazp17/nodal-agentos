//! `ChatSink` implementation. P05: each chat the frontend has attached gets its own
//! `tauri::ipc::Channel`, so a chat's stream reaches only the listener that asked for it
//! instead of every window hearing every chat's `nodal://chat` broadcast. `emit` falls back to
//! the old broadcast for a chat with no attached channel yet, which shouldn't happen in
//! practice: the frontend awaits `attach_chat_channel` before it can start a chat's first turn
//! (see `src/features/chats/stream.ts`'s `ensureChatAttached`).

use std::collections::HashMap;
use std::sync::Mutex;

use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Runtime, Wry};

use nodal_domain::model::chat::{ChatEnvelope, CHAT_EVENT};
use nodal_domain::ports::ChatSink;

pub struct TauriChatSink<R: Runtime = Wry> {
    app: AppHandle<R>,
    channels: Mutex<HashMap<String, Channel<ChatEnvelope>>>,
}

impl<R: Runtime> TauriChatSink<R> {
    pub fn new(app: AppHandle<R>) -> Self {
        Self {
            app,
            channels: Mutex::new(HashMap::new()),
        }
    }

    /// Registers (or replaces) `chat_id`'s channel.
    pub fn attach(&self, chat_id: String, channel: Channel<ChatEnvelope>) {
        self.channels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(chat_id, channel);
    }

    /// Stops delivering to `chat_id` (the chat was deleted, or its channel closed).
    pub fn detach(&self, chat_id: &str) {
        self.channels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(chat_id);
    }
}

impl<R: Runtime> ChatSink for TauriChatSink<R> {
    fn emit(&self, ev: ChatEnvelope) {
        crate::telemetry::record_chat_emit();
        let channel = self
            .channels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&ev.chat_id)
            .cloned();
        match channel {
            Some(ch) => {
                if let Err(e) = ch.send(ev) {
                    eprintln!("chats: {e}");
                }
            }
            None => {
                if let Err(e) = self.app.emit(CHAT_EVENT, ev) {
                    eprintln!("chats: {e}");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
