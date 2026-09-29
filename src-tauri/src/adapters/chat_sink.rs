//! `ChatSink` implementation: emits `nodal://chat` (`ChatEnvelope`) to the webview. Built by
//! `commands::chats::runtime`.

use tauri::{AppHandle, Emitter};

use nodal_domain::model::chat::{ChatEnvelope, CHAT_EVENT};
use nodal_domain::ports::ChatSink;

pub struct TauriChatSink(AppHandle);

impl TauriChatSink {
    pub fn new(app: AppHandle) -> Self {
        Self(app)
    }
}

impl ChatSink for TauriChatSink {
    fn emit(&self, ev: ChatEnvelope) {
        crate::telemetry::record_chat_emit();
        if let Err(e) = self.0.emit(CHAT_EVENT, ev) {
            eprintln!("chats: {e}");
        }
    }
}
