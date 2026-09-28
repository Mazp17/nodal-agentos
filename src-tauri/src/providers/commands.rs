//! Moved to `SRC/commands/sources.rs`; re-exported so current uses don't break.

/// Only the bridge is left; nothing in this crate calls them directly anymore.
#[allow(unused_imports)]
pub use crate::commands::sources::{
    create_source_link, delete_source_link, import_rule, import_tasks, list_source_links, preview_rule_import,
    provider_clear_key, provider_list_importable, provider_scopes, provider_set_key, provider_status,
    resolve_moved_task, save_state_map, source_rule_projects, source_states, sync_now, unlink_task, update_source_link,
};
