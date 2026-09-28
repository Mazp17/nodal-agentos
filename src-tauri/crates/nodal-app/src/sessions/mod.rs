//! Sessions context: read-only access to Claude Code sessions and runs' transcripts. Filled
//! in wave 3a.

use std::sync::Arc;

use nodal_domain::ports::{ClaudeCli, SessionFiles};

use crate::core::Core;

/// Serves `list_runs`, `get_run_detail`, `get_agent_transcript` and `get_launch_blocker`
/// without a database, exactly like today; always registered.
#[allow(dead_code)]
pub struct SessionReader {
    claude: Arc<dyn ClaudeCli>,
    files: Arc<dyn SessionFiles>,
}

impl SessionReader {
    pub fn new(claude: Arc<dyn ClaudeCli>, files: Arc<dyn SessionFiles>) -> Arc<Self> {
        Arc::new(Self { claude, files })
    }
}

/// Filled in wave 3a; holds `core` for `get_run_transcript`/`external_sessions`, which need
/// the database.
#[allow(dead_code)]
pub struct Sessions {
    core: Arc<Core>,
}

impl Sessions {
    pub(crate) fn new(core: Arc<Core>) -> Self {
        Self { core }
    }
}
