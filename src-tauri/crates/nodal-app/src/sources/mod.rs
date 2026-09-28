//! Sources context: task providers (Linear today), sync and their DB-free hub. Filled in
//! wave 3b.

pub mod keys;
pub mod registry;

use std::path::PathBuf;
use std::sync::Arc;

use tokio::runtime::Handle;

use nodal_domain::ports::{ChangeNotifier, Clock, PlanFiles};
use nodal_store::Db;

use crate::core::HubDeps;

/// Providers, keys and sync state, usable without a database (today's `ProvidersState` +
/// `Secrets` + `LinearState`): `SourcesHub::new` never fails.
#[allow(dead_code)]
pub struct SourcesHub {
    pub db: Option<Db>,
    pub data_dir: PathBuf,
    rt: Handle,
    clock: Arc<dyn Clock>,
    pub secrets: keys::Secrets,
    pub providers: registry::ProviderRegistry,
    notifier: Arc<dyn ChangeNotifier>,
    plans: Arc<dyn PlanFiles>,
}

impl SourcesHub {
    pub fn new(d: HubDeps) -> Arc<Self> {
        Arc::new(Self {
            db: d.db,
            data_dir: d.data_dir,
            rt: d.rt,
            clock: d.clock,
            secrets: d.secrets,
            providers: d.providers,
            notifier: d.notifier,
            plans: d.plans,
        })
    }
}
