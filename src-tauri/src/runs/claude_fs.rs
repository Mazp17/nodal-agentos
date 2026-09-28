//! WARNING: Claude Code's internal, UNDOCUMENTED format. Moved to `nodal_host::claude::fs`
//! (split by concern into `paths`, `bg_output`, `agents_json`, `workflow_detail`,
//! `js_script`, `last_tool`, `transcript`, `usage`, `review_denial`, `readout`);
//! re-exported here so current uses don't break.

pub use nodal_domain::sessions::transcript::{
    is_valid_path_id, is_valid_session_id, user_item, TRANSCRIPT_DEFAULT_LIMIT, TRANSCRIPT_MAX_LIMIT,
};
/// Only the bridge is left; nothing in this crate calls them directly anymore.
#[allow(unused_imports)]
pub use nodal_domain::sessions::transcript::{clip, TEXT_MAX};

pub use nodal_host::claude::fs::paths::{claude_config_dir, find_session_dir, find_session_jsonl};
pub use nodal_host::claude::fs::review_denial::read_workflow_review_denial;
pub use nodal_host::claude::fs::transcript::{read_agent_transcript, read_session_transcript, session_title};
pub use nodal_host::claude::fs::workflow_detail::read_run_detail;

/// Only the bridge is left; nothing in this crate calls them directly anymore.
#[allow(unused_imports)]
pub use nodal_host::claude::fs::agents_json::parse_agents_json;
#[allow(unused_imports)]
pub use nodal_host::claude::fs::bg_output::{parse_bare_id, parse_bg_line};
#[allow(unused_imports)]
pub use nodal_host::claude::fs::js_script::parse_script_phases;
#[allow(unused_imports)]
pub use nodal_host::claude::fs::last_tool::{last_tool_from_transcript, last_tool_in_lines};
#[allow(unused_imports)]
pub use nodal_host::claude::fs::paths::project_slug;
#[allow(unused_imports)]
pub use nodal_host::claude::fs::readout::{projects_dir, read_session, session_tokens};
#[allow(unused_imports)]
pub use nodal_host::claude::fs::review_denial::{find_workflow_review_denial, last_assistant_text_in, read_last_assistant_text, WORKFLOW_REVIEW_TEXT};
#[allow(unused_imports)]
pub use nodal_host::claude::fs::transcript::{parse_transcript, ParsedTranscript};
#[allow(unused_imports)]
pub use nodal_host::claude::fs::usage::{read_usage_tokens, usage_tokens_in};
#[allow(unused_imports)]
pub use nodal_host::claude::fs::workflow_detail::{parse_final, parse_journal, parse_result};
