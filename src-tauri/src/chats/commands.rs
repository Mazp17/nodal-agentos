//! Moved to `SRC/commands/chats.rs`; re-exported so current uses don't break.

/// Only the bridge is left; nothing in this crate calls them directly anymore.
#[allow(unused_imports)]
pub use crate::commands::chats::{
    create_chat, delete_chat, get_chat_live, get_chat_transcript, get_claude_defaults, interrupt_chat, list_chats,
    respond_chat_permission, send_chat_message, stop_chat, update_chat,
};
