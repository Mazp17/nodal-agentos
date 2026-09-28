//! Fake provider for tests: in-memory items and states, a log of writes and programmable
//! failures.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use nodal_domain::model::providers::Page;
use nodal_domain::ports::BoxFut;

use crate::domain::{ExternalState, ScopeRef};

use super::{iso_from_ms, ErrorKind, ExternalItem, ImportQuery, ProviderError, ProviderResult, TaskProvider};

#[derive(Default)]
pub struct FakeData {
    pub states: Vec<ExternalState>,
    pub items: BTreeMap<String, ExternalItem>,
    /// If set, `set_state` and `comment` fail with this error.
    pub fail_writes: Option<ProviderError>,
    /// If set, `states` fails with this error.
    pub fail_states: Option<ProviderError>,
    pub set_states: Vec<(String, String)>,
    pub comments: Vec<(String, String)>,
    /// Calls to `states`.
    pub states_calls: usize,
    /// Projects returned by `rule_projects`.
    pub projects: Vec<ScopeRef>,
    /// Provider's "now" (epoch ms) for `closed_within_days`.
    pub now: i64,
    /// Queries received by `list_importable`.
    pub queries: Vec<ImportQuery>,
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

    fn status(&self) -> BoxFut<'_, ProviderResult<String>> {
        Box::pin(async move { Ok("Fake User".into()) })
    }

    fn scopes(&self) -> BoxFut<'_, ProviderResult<Vec<ScopeRef>>> {
        Box::pin(async move {
            Ok(vec![ScopeRef { kind: "team".into(), id: "team-eng".into(), name: "Engineering".into() }])
        })
    }

    fn states<'a>(&'a self, _scope: &'a ScopeRef) -> BoxFut<'a, ProviderResult<Vec<ExternalState>>> {
        Box::pin(async move {
            let mut d = self.data();
            d.states_calls += 1;
            match &d.fail_states {
                Some(e) => Err(e.clone()),
                None => Ok(d.states.clone()),
            }
        })
    }

    fn rule_projects<'a>(&'a self, _scope: &'a ScopeRef) -> BoxFut<'a, ProviderResult<Vec<ScopeRef>>> {
        Box::pin(async move { Ok(self.data().projects.clone()) })
    }

    /// Emulates Linear's filter: state types (or closed less than `closed_within_days`
    /// ago), text, project and `created_after` (comparable ISO dates).
    fn list_importable<'a>(&'a self, q: &'a ImportQuery) -> BoxFut<'a, ProviderResult<Page>> {
        Box::pin(async move {
            let mut d = self.data();
            d.queries.push(q.clone());
            let since = q.closed_within_days.map(|days| iso_from_ms(d.now - i64::from(days) * 86_400_000));
            let created = q.created_after.map(iso_from_ms);
            let items = d
                .items
                .values()
                .filter(|i| {
                    let open = q.state_kinds.is_empty() || q.state_kinds.contains(&i.state.kind);
                    let recent = since.as_ref().is_some_and(|s| i.closed_at.as_ref().is_some_and(|c| c > s));
                    open || recent
                })
                .filter(|i| q.text.as_deref().is_none_or(|t| i.title.to_lowercase().contains(&t.to_lowercase())))
                .filter(|i| q.project_id.as_ref().is_none_or(|p| i.project().is_some_and(|ip| &ip.id == p)))
                .filter(|i| created.as_ref().is_none_or(|c| i.created_at.as_ref().is_some_and(|ic| ic > c)))
                .cloned()
                .collect();
            Ok(Page { items, next_cursor: None })
        })
    }

    fn pull<'a>(&'a self, ids: &'a [String]) -> BoxFut<'a, ProviderResult<Vec<ExternalItem>>> {
        Box::pin(async move {
            let d = self.data();
            Ok(ids.iter().filter_map(|id| d.items.get(id).cloned()).collect())
        })
    }

    fn set_state<'a>(&'a self, external_id: &'a str, state_id: &'a str) -> BoxFut<'a, ProviderResult<ExternalState>> {
        Box::pin(async move {
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
        })
    }

    fn comment<'a>(&'a self, external_id: &'a str, body: &'a str) -> BoxFut<'a, ProviderResult<()>> {
        Box::pin(async move {
            let mut d = self.data();
            if let Some(e) = &d.fail_writes {
                return Err(e.clone());
            }
            d.comments.push((external_id.into(), body.into()));
            Ok(())
        })
    }

    fn has_comment_with<'a>(&'a self, external_id: &'a str, marker: &'a str) -> BoxFut<'a, ProviderResult<bool>> {
        Box::pin(async move {
            let d = self.data();
            Ok(d.comments.iter().any(|(id, body)| id == external_id && body.contains(marker)))
        })
    }
}
