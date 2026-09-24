//! Proveedor falso para tests: ítems y estados en memoria, registro de escrituras y fallos
//! programables.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::domain::{ExternalState, ScopeRef};

use super::{ErrorKind, ExternalItem, ImportQuery, Page, ProviderError, ProviderResult, TaskProvider};

#[derive(Default)]
pub struct FakeData {
    pub states: Vec<ExternalState>,
    pub items: BTreeMap<String, ExternalItem>,
    /// Si está, `set_state` y `comment` fallan con este error.
    pub fail_writes: Option<ProviderError>,
    /// Si está, `states` falla con este error.
    pub fail_states: Option<ProviderError>,
    pub set_states: Vec<(String, String)>,
    pub comments: Vec<(String, String)>,
    /// Llamadas a `states`.
    pub states_calls: usize,
}

#[derive(Clone, Default)]
pub struct FakeProvider(pub Arc<Mutex<FakeData>>);

impl FakeProvider {
    pub fn data(&self) -> MutexGuard<'_, FakeData> {
        self.0.lock().unwrap()
    }
}

impl TaskProvider for FakeProvider {
    fn name(&self) -> &'static str {
        "fake"
    }

    async fn status(&self) -> ProviderResult<String> {
        Ok("Fake User".into())
    }

    async fn scopes(&self) -> ProviderResult<Vec<ScopeRef>> {
        Ok(vec![ScopeRef { kind: "team".into(), id: "team-eng".into(), name: "Engineering".into() }])
    }

    async fn states(&self, _scope: &ScopeRef) -> ProviderResult<Vec<ExternalState>> {
        let mut d = self.data();
        d.states_calls += 1;
        match &d.fail_states {
            Some(e) => Err(e.clone()),
            None => Ok(d.states.clone()),
        }
    }

    async fn list_importable(&self, q: &ImportQuery) -> ProviderResult<Page> {
        let d = self.data();
        let items = d
            .items
            .values()
            .filter(|i| q.state_kinds.is_empty() || q.state_kinds.contains(&i.state.kind))
            .filter(|i| q.text.as_deref().is_none_or(|t| i.title.to_lowercase().contains(&t.to_lowercase())))
            .cloned()
            .collect();
        Ok(Page { items, next_cursor: None })
    }

    async fn pull(&self, ids: &[String]) -> ProviderResult<Vec<ExternalItem>> {
        let d = self.data();
        Ok(ids.iter().filter_map(|id| d.items.get(id).cloned()).collect())
    }

    async fn set_state(&self, external_id: &str, state_id: &str) -> ProviderResult<ExternalState> {
        let mut d = self.data();
        if let Some(e) = &d.fail_writes {
            return Err(e.clone());
        }
        let state = d
            .states
            .iter()
            .find(|s| s.id == state_id)
            .cloned()
            .ok_or_else(|| ProviderError::new(ErrorKind::Permanent, "unknown state"))?;
        if let Some(i) = d.items.get_mut(external_id) {
            i.state = state.clone();
        }
        d.set_states.push((external_id.into(), state_id.into()));
        Ok(state)
    }

    async fn comment(&self, external_id: &str, body: &str) -> ProviderResult<()> {
        let mut d = self.data();
        if let Some(e) = &d.fail_writes {
            return Err(e.clone());
        }
        d.comments.push((external_id.into(), body.into()));
        Ok(())
    }

    async fn has_comment_with(&self, external_id: &str, marker: &str) -> ProviderResult<bool> {
        let d = self.data();
        Ok(d.comments.iter().any(|(id, body)| id == external_id && body.contains(marker)))
    }
}
