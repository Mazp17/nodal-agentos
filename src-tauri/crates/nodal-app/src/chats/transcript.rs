//! Turns a chat's raw session transcript into what the UI expects (was
//! `chats::commands::with_first_message`).

use nodal_domain::model::claude::{Transcript, TranscriptItem};
use nodal_domain::sessions::transcript::user_item;

/// The transcript keeps the messages before the first answer apart as `prompt`; in a chat
/// it is the first user message, so it goes back into the items (when none are omitted).
pub(super) fn with_first_message(mut t: Transcript) -> Transcript {
    if t.omitted == 0 {
        let images = std::mem::take(&mut t.prompt_images);
        if t.prompt.is_some() || !images.is_empty() {
            let mut first = user_item(t.prompt.as_deref().unwrap_or(""), t.prompt_uuid.take());
            if let TranscriptItem::User { images: slot, .. } = &mut first {
                *slot = images;
            }
            t.prompt = None;
            t.items.insert(0, first);
            t.total_items += 1;
        }
    }
    t
}
