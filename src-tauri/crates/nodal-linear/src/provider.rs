//! Linear adapter. Queries and parsing live in `crate::{client,model}`; here we only
//! translate to the generic model. Scopes: `team` and `project`.

use std::sync::Arc;

use nodal_domain::model::providers::{
    ChildItem, ErrorKind, ExternalItem, ImportQuery, ItemRef, Page, ProviderError, ProviderResult,
};
use nodal_domain::model::{ExtKind, ExternalState, Priority, ScopeRef};
use nodal_domain::ports::{BoxFut, ProviderFactory, TaskProvider};
use nodal_domain::sources::iso_from_ms;

use crate::client::LinearClient;
use crate::model::{importable_filter, pick_team_state, project_rule_filter, ScopedState, SyncIssue, WorkflowState};

pub struct LinearProvider {
    http: reqwest::Client,
    key: String,
}

impl LinearProvider {
    pub fn new(http: reqwest::Client, key: String) -> Self {
        Self { http, key }
    }

    fn client(&self) -> LinearClient<'_> {
        LinearClient::new(&self.http, &self.key)
    }
}

/// `WorkflowState.type` → normalized kind.
pub fn ext_kind(state_type: &str) -> ExtKind {
    match state_type {
        "triage" => ExtKind::Triage,
        "backlog" => ExtKind::Backlog,
        "unstarted" => ExtKind::Unstarted,
        "started" => ExtKind::Started,
        "completed" => ExtKind::Completed,
        // In case Linear exposes "duplicate" as its own type (today it is `canceled`).
        "canceled" | "duplicate" => ExtKind::Canceled,
        _ => ExtKind::Unknown,
    }
}

fn linear_type(kind: ExtKind) -> Option<&'static str> {
    Some(match kind {
        ExtKind::Triage => "triage",
        ExtKind::Backlog => "backlog",
        ExtKind::Unstarted => "unstarted",
        ExtKind::Started => "started",
        ExtKind::Completed => "completed",
        ExtKind::Canceled => "canceled",
        ExtKind::Unknown => return None,
    })
}

pub fn ext_state(s: &WorkflowState) -> ExternalState {
    ExternalState { id: s.id.clone(), name: s.name.clone(), kind: ext_kind(&s.state_type), color: Some(s.color.clone()) }
}

/// 0 = no priority, 1 = urgent … 4 = low.
pub fn priority(p: f64) -> Priority {
    match p.round() as i64 {
        1 => Priority::Urgent,
        2 => Priority::High,
        3 => Priority::Medium,
        4 => Priority::Low,
        _ => Priority::None,
    }
}

/// States of one or more teams. With several (multi-team project) the name carries the team
/// key to tell "ENG · In Progress" apart from "OPS · In Progress".
pub fn scoped_states(states: &[ScopedState]) -> Vec<ExternalState> {
    let multi = states.iter().any(|s| s.team.id != states[0].team.id);
    states
        .iter()
        .map(|s| {
            let mut e = ext_state(&s.state);
            if multi {
                e.name = format!("{} · {}", s.team.key, e.name);
            }
            e
        })
        .collect()
}

pub fn to_item(i: SyncIssue) -> ExternalItem {
    let mut scopes = vec![ScopeRef { kind: "team".into(), id: i.team.id.clone(), name: i.team.name.clone() }];
    if let Some(p) = &i.project {
        scopes.push(ScopeRef { kind: "project".into(), id: p.id.clone(), name: p.name.clone() });
    }
    ExternalItem {
        external_id: i.id,
        identifier: i.identifier,
        url: i.url,
        title: i.title,
        description_md: i.description.filter(|d| !d.trim().is_empty()),
        state: ext_state(&i.state),
        scopes,
        parent: i.parent.map(|p| ItemRef { external_id: p.id, identifier: p.identifier, title: p.title, url: p.url }),
        children: i
            .children
            .map(|c| c.nodes)
            .unwrap_or_default()
            .into_iter()
            .map(|c| ChildItem {
                external_id: c.id,
                identifier: c.identifier,
                title: c.title,
                url: c.url,
                state: ext_state(&c.state),
            })
            .collect(),
        labels: i.labels.nodes.into_iter().map(|l| l.name).collect(),
        assignee: i.assignee.map(|a| a.name),
        priority: priority(i.priority),
        updated_at: i.updated_at,
        created_at: i.created_at,
        closed_at: i.completed_at.or(i.canceled_at),
    }
}

impl TaskProvider for LinearProvider {
    fn name(&self) -> &'static str {
        "linear"
    }

    fn status(&self) -> BoxFut<'_, ProviderResult<String>> {
        Box::pin(async move { Ok(self.client().viewer().await?.name) })
    }

    fn scopes(&self) -> BoxFut<'_, ProviderResult<Vec<ScopeRef>>> {
        Box::pin(async move {
            let c = self.client();
            let teams = c.teams().await?;
            let projects = c.projects().await?;
            Ok(teams
                .into_iter()
                .map(|t| ScopeRef { kind: "team".into(), id: t.id, name: t.name })
                .chain(projects.into_iter().map(|p| ScopeRef { kind: "project".into(), id: p.id, name: p.name }))
                .collect())
        })
    }

    fn states<'a>(&'a self, scope: &'a ScopeRef) -> BoxFut<'a, ProviderResult<Vec<ExternalState>>> {
        Box::pin(async move {
            let c = self.client();
            let team_ids = match scope.kind.as_str() {
                "team" => vec![scope.id.clone()],
                "project" => c.project_teams(&scope.id).await?.into_iter().map(|t| t.id).collect(),
                other => return Err(ProviderError::new(ErrorKind::Permanent, format!("Unsupported Linear scope \"{other}\"."))),
            };
            if team_ids.is_empty() {
                return Ok(Vec::new());
            }
            let states = c.workflow_states(&team_ids).await?;
            Ok(if states.is_empty() { Vec::new() } else { scoped_states(&states) })
        })
    }

    fn rule_projects<'a>(&'a self, scope: &'a ScopeRef) -> BoxFut<'a, ProviderResult<Vec<ScopeRef>>> {
        Box::pin(async move {
            match scope.kind.as_str() {
                "team" => Ok(self
                    .client()
                    .team_projects(&scope.id)
                    .await?
                    .into_iter()
                    .map(|p| ScopeRef { kind: "project".into(), id: p.id, name: p.name })
                    .collect()),
                "project" => Ok(vec![scope.clone()]),
                other => Err(ProviderError::new(ErrorKind::Permanent, format!("Unsupported Linear scope \"{other}\"."))),
            }
        })
    }

    fn list_importable<'a>(&'a self, q: &'a ImportQuery) -> BoxFut<'a, ProviderResult<Page>> {
        Box::pin(async move {
            let types: Vec<&str> = q.state_kinds.iter().filter_map(|k| linear_type(*k)).collect();
            let created = q.created_after.map(iso_from_ms);
            let filter = match &q.project_id {
                Some(project) => project_rule_filter(
                    &q.scope.kind,
                    &q.scope.id,
                    project,
                    &types,
                    q.closed_within_days,
                    created.as_deref(),
                ),
                None => importable_filter(&q.scope.kind, &q.scope.id, &types, q.text.as_deref(), created.as_deref()),
            };
            let (issues, next) =
                self.client().importable_page(&filter, q.cursor.as_deref()).await?;
            Ok(Page { items: issues.into_iter().map(to_item).collect(), next_cursor: next })
        })
    }

    fn pull<'a>(&'a self, external_ids: &'a [String]) -> BoxFut<'a, ProviderResult<Vec<ExternalItem>>> {
        Box::pin(async move {
            if external_ids.is_empty() {
                return Ok(Vec::new());
            }
            let issues = self.client().issues_by_ids(external_ids).await?;
            Ok(issues.into_iter().map(to_item).collect())
        })
    }

    fn set_state<'a>(&'a self, external_id: &'a str, state_id: &'a str) -> BoxFut<'a, ProviderResult<ExternalState>> {
        Box::pin(async move {
            let c = self.client();
            // One extra query per push (there are few): in a multi-team project the mapping points
            // to one team's state, and the issue may belong to another.
            let (team, target) = c.issue_team_states(external_id, state_id).await?;
            let Some(pick) = pick_team_state(&team, &target) else {
                return Err(ProviderError::new(
                    ErrorKind::Permanent,
                    format!("The issue's team has no state like \"{}\". Review the mapping.", target.name),
                ));
            };
            Ok(ext_state(&c.set_issue_state(external_id, &pick.id).await?))
        })
    }

    fn comment<'a>(&'a self, external_id: &'a str, body: &'a str) -> BoxFut<'a, ProviderResult<()>> {
        Box::pin(async move { Ok(self.client().create_comment(external_id, body).await?) })
    }

    fn has_comment_with<'a>(&'a self, external_id: &'a str, marker: &'a str) -> BoxFut<'a, ProviderResult<bool>> {
        Box::pin(async move { Ok(self.client().has_comment_with(external_id, marker).await?) })
    }
}

/// Builds a `LinearProvider` bound to a saved key, sharing one HTTP client.
pub struct LinearFactory {
    http: reqwest::Client,
}

impl LinearFactory {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }
}

impl ProviderFactory for LinearFactory {
    fn name(&self) -> &'static str {
        "linear"
    }

    fn build(&self, key: String) -> Arc<dyn TaskProvider> {
        Arc::new(LinearProvider::new(self.http.clone(), key))
    }
}

#[cfg(test)]
mod tests;
