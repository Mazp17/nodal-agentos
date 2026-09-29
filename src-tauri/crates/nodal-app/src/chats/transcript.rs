//! Turns a chat's raw session transcript into what the UI expects (was
//! `chats::commands::with_first_message`).

use nodal_domain::model::claude::Transcript;
use nodal_domain::sessions::transcript::user_item;

/// The transcript keeps the messages before the first answer apart as `prompt`; in a chat
/// it is the first user message, so it goes back into the items (when none are omitted).
pub(super) fn with_first_message(mut t: Transcript) -> Transcript {
    if t.omitted == 0 {
        if let Some(p) = t.prompt.take() {
            t.items.insert(0, user_item(&p));
            t.total_items += 1;
        }
    }
    t
}
