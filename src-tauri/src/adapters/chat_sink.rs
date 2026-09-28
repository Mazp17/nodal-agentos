//! `ChatSink` implementation: emits `nodal://chat` (`ChatEnvelope`) to the webview. Ready
//! for wave 3b's `commands::chats::runtime`; today's setup still goes through the legacy
//! `chats::process::Emit` closure in `SRC/lib.rs`.

use tauri::{AppHandle, Emitter};

use nodal_domain::model::chat::{ChatEnvelope, CHAT_EVENT};
use nodal_domain::ports::ChatSink;

/// Not built yet: wave 3b's `commands::chats::runtime` will construct one instead of going
/// through the legacy `Emit` closure.
#[allow(dead_code)]
pub struct TauriChatSink(AppHandle);

#[allow(dead_code)]
impl TauriChatSink {
    pub fn new(app: AppHandle) -> Self {
        Self(app)
    }
}

impl ChatSink for TauriChatSink {
    fn emit(&self, ev: ChatEnvelope) {
        if let Err(e) = self.0.emit(CHAT_EVENT, ev) {
            eprintln!("chats: {e}");
        }
    }
}
