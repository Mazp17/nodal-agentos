//! Linear adapter. Queries and parsing live in `crate::linear::{client,model}`; here we only
//! translate to the generic model. Scopes: `team` and `project`.

use crate::domain::{ExtKind, ExternalState, Priority, ScopeRef};
use crate::linear::client::LinearClient;
use crate::linear::model::{importable_filter, pick_team_state, project_rule_filter, ScopedState, SyncIssue, WorkflowState};

use super::{iso_from_ms, ChildItem, ErrorKind, ExternalItem, ImportQuery, ItemRef, Page, ProviderError, ProviderResult, TaskProvider};

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

    async fn status(&self) -> ProviderResult<String> {
        Ok(self.client().viewer().await?.name)
    }

    async fn scopes(&self) -> ProviderResult<Vec<ScopeRef>> {
        let c = self.client();
        let teams = c.teams().await?;
        let projects = c.projects().await?;
        Ok(teams
            .into_iter()
            .map(|t| ScopeRef { kind: "team".into(), id: t.id, name: t.name })
            .chain(projects.into_iter().map(|p| ScopeRef { kind: "project".into(), id: p.id, name: p.name }))
            .collect())
    }

    async fn states(&self, scope: &ScopeRef) -> ProviderResult<Vec<ExternalState>> {
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
    }

    async fn rule_projects(&self, scope: &ScopeRef) -> ProviderResult<Vec<ScopeRef>> {
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
    }

    async fn list_importable(&self, q: &ImportQuery) -> ProviderResult<Page> {
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
    }

    async fn pull(&self, external_ids: &[String]) -> ProviderResult<Vec<ExternalItem>> {
        if external_ids.is_empty() {
            return Ok(Vec::new());
        }
        let issues = self.client().issues_by_ids(external_ids).await?;
        Ok(issues.into_iter().map(to_item).collect())
    }

    async fn set_state(&self, external_id: &str, state_id: &str) -> ProviderResult<ExternalState> {
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
    }

    async fn comment(&self, external_id: &str, body: &str) -> ProviderResult<()> {
        Ok(self.client().create_comment(external_id, body).await?)
    }

    async fn has_comment_with(&self, external_id: &str, marker: &str) -> ProviderResult<bool> {
        Ok(self.client().has_comment_with(external_id, marker).await?)
    }
}

#[cfg(test)]
mod tests;
