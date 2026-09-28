//! Chats context: per-project `claude -p` sessions. Filled in wave 3b.

pub mod hooks;

use std::sync::Arc;

use crate::core::Core;

/// Filled in wave 3b; holds `core` for the 11 chat commands.
#[allow(dead_code)]
pub struct Chats {
    core: Arc<Core>,
}

impl Chats {
    pub(crate) fn new(core: Arc<Core>) -> Self {
        Self { core }
    }
}
