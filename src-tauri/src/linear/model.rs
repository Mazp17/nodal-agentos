//! Types, queries and parsing of Linear GraphQL responses. No network: everything here is
//! pure so it can be tested with fixtures.

use super::error::LinearError;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Viewer {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Team {
    pub id: String,
    pub key: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowState {
    pub id: String,
    pub name: String,
    /// triage | backlog | unstarted | started | completed | canceled
    #[serde(rename = "type")]
    pub state_type: String,
    pub position: f64,
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserRef {
    pub id: String,
    pub name: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRef {
    pub id: String,
    pub name: String,
}

// ---- Raw GraphQL response shapes ----

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Connection<T> {
    pub nodes: Vec<T>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageInfo {
    pub has_next_page: bool,
    pub end_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PagedConnection<T> {
    pub nodes: Vec<T>,
    pub page_info: PageInfo,
}

#[derive(Debug, Deserialize)]
pub struct ViewerData {
    pub viewer: Viewer,
}

#[derive(Debug, Deserialize)]
pub struct TeamsData {
    pub teams: Connection<Team>,
}

// ---- Queries ----

pub const VIEWER_QUERY: &str = "query Viewer { viewer { name email } }";

pub const TEAMS_QUERY: &str = "query Teams { teams(first: 250) { nodes { id key name } } }";

// ---- Sync with Nodal (providers::linear) ----
//
// Fields and arguments verified against the official SDK schema
// (github.com/linear/linear, packages/sdk/src/schema.graphql, 2026-09):
// - `Query.issues(filter: IssueFilter, first, after, includeArchived, orderBy)`,
//   `Query.workflowStates(filter: WorkflowStateFilter, first)`, `Query.project(id: String!)`,
//   `Query.projects(filter: ProjectFilter, first)`;
// - `IssueFilter.{id: IssueIDComparator{in: [ID!]}, team: TeamFilter, project:
//   NullableProjectFilter{id}, state: WorkflowStateFilter{type}, title: StringComparator
//   {containsIgnoreCase}, number: NumberComparator{eq}, createdAt: DateComparator{gt}, or}`;
// - `ProjectFilter.status: ProjectStatusFilter{type}` (`ProjectStatusType`: backlog, planned,
//   started, paused, completed, canceled);
// - `Issue.{id, identifier, title, url, description (markdown), priority: Float!, updatedAt,
//   state, team, project, assignee, parent, labels(first), children(first)}`;
// - `Mutation.issueUpdate(id: String!, input: IssueUpdateInput{stateId})` and
//   `Mutation.commentCreate(input: CommentCreateInput{issueId, body})`, both with `success`
//   (`IssuePayload.issue` carries the resulting state);
// - `Query.workflowState(id: String!)` and `Team.states(first)` to resolve the push.
// Project rules (same schema, 2026-09-24):
// - `Issue.{project: Project {id, name}, createdAt: DateTime!, completedAt: DateTime,
//   canceledAt: DateTime}`;
// - `IssueFilter.{completedAt, canceledAt}: NullableDateComparator{gt: DateTimeOrDuration}`;
// - `ProjectFilter.accessibleTeams: TeamCollectionFilter{some: TeamFilter{id}}`.

/// Page of the importables listing. Small on purpose: each issue carries `labels(first: 20)`.
pub const SYNC_PAGE_SIZE: u32 = 25;
/// Ids per `issues_by_ids` query. With `children(50)` + `labels(20)` per issue the query
/// lands at a few thousand complexity points (estimated, not measured), under the 10k limit.
pub const PULL_BATCH: usize = 25;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct NameNode {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncIssueRef {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncChild {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub url: String,
    pub state: WorkflowState,
}

/// Issue as used by the sync. The listing does not request `description`, `parent`,
/// `children` or `assignee`: they arrive empty.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncIssue {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub url: String,
    pub priority: f64,
    pub updated_at: String,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub canceled_at: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    pub state: WorkflowState,
    pub team: Team,
    #[serde(default)]
    pub project: Option<ProjectRef>,
    #[serde(default)]
    pub assignee: Option<UserRef>,
    #[serde(default)]
    pub parent: Option<SyncIssueRef>,
    pub labels: Connection<NameNode>,
    #[serde(default)]
    pub children: Option<Connection<SyncChild>>,
}

#[derive(Debug, Deserialize)]
pub struct SyncIssuesData {
    pub issues: PagedConnection<SyncIssue>,
}

#[derive(Debug, Deserialize)]
pub struct ProjectsData {
    pub projects: Connection<ProjectRef>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ScopedState {
    #[serde(flatten)]
    pub state: WorkflowState,
    pub team: Team,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStatesData {
    pub workflow_states: Connection<ScopedState>,
}

#[derive(Debug, Deserialize)]
pub struct ProjectTeams {
    pub teams: Connection<Team>,
}

#[derive(Debug, Deserialize)]
pub struct ProjectTeamsData {
    pub project: ProjectTeams,
}

#[derive(Debug, Deserialize)]
pub struct Success {
    pub success: bool,
}

#[derive(Debug, Deserialize)]
pub struct IssueStateRef {
    pub state: WorkflowState,
}

#[derive(Debug, Deserialize)]
pub struct IssueUpdatePayload {
    pub success: bool,
    pub issue: Option<IssueStateRef>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueUpdateData {
    pub issue_update: IssueUpdatePayload,
}

#[derive(Debug, Deserialize)]
pub struct TeamWithStates {
    pub states: Connection<WorkflowState>,
}

#[derive(Debug, Deserialize)]
pub struct IssueTeam {
    pub team: TeamWithStates,
}

/// States of the issue's team and the target state, to resolve the push (see
/// `pick_team_state`).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueTeamStatesData {
    pub issue: IssueTeam,
    pub workflow_state: WorkflowState,
}

/// State of the issue's team equivalent to `target`: the same id; otherwise (the mapping is
/// from another team, in a multi-team project), the one with the same name and type;
/// otherwise, the one with the same name.
pub fn pick_team_state<'a>(team: &'a [WorkflowState], target: &WorkflowState) -> Option<&'a WorkflowState> {
    let same_name = |s: &&WorkflowState| s.name.trim().eq_ignore_ascii_case(target.name.trim());
    team.iter()
        .find(|s| s.id == target.id)
        .or_else(|| team.iter().filter(same_name).find(|s| s.state_type == target.state_type))
        .or_else(|| team.iter().find(same_name))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentCreateData {
    pub comment_create: Success,
}

#[derive(Debug, Deserialize)]
pub struct IdNode {
    #[allow(dead_code)] // Only matters whether there are nodes.
    pub id: String,
}

#[derive(Debug, Deserialize)]
pub struct IssueComments {
    pub comments: Connection<IdNode>,
}

#[derive(Debug, Deserialize)]
pub struct CommentMarkerData {
    /// `null` if the issue no longer exists or the key cannot see it.
    pub issue: Option<IssueComments>,
}

pub const PROJECTS_QUERY: &str = r#"query NodalProjects {
  projects(first: 100, filter: { status: { type: { nin: ["completed", "canceled"] } } }) {
    nodes { id name }
  }
}"#;

/// Active projects a team has access to (to pick the project rule).
pub const TEAM_PROJECTS_QUERY: &str = r#"query NodalTeamProjects($teamId: ID!) {
  projects(first: 100, filter: {
    accessibleTeams: { some: { id: { eq: $teamId } } },
    status: { type: { nin: ["completed", "canceled"] } }
  }) {
    nodes { id name }
  }
}"#;

pub const PROJECT_TEAMS_QUERY: &str = "query NodalProjectTeams($id: String!) {
  project(id: $id) { teams(first: 20) { nodes { id key name } } }
}";

pub const WORKFLOW_STATES_QUERY: &str = "query NodalStates($teamIds: [ID!]) {
  workflowStates(first: 100, filter: { team: { id: { in: $teamIds } } }) {
    nodes { id name type position color team { id key name } }
  }
}";

pub const IMPORTABLE_QUERY: &str = "query NodalImportable($filter: IssueFilter, $first: Int!, $after: String) {
  issues(filter: $filter, first: $first, after: $after, orderBy: updatedAt) {
    nodes {
      id identifier title url priority updatedAt createdAt completedAt canceledAt
      state { id name type position color }
      team { id key name }
      project { id name }
      labels(first: 20) { nodes { name } }
    }
    pageInfo { hasNextPage endCursor }
  }
}";

pub const ISSUES_BY_IDS_QUERY: &str = "query NodalIssues($ids: [ID!], $first: Int!) {
  issues(filter: { id: { in: $ids } }, first: $first, includeArchived: true) {
    nodes {
      id identifier title url priority updatedAt createdAt completedAt canceledAt description
      state { id name type position color }
      team { id key name }
      project { id name }
      assignee { id name displayName }
      parent { id identifier title url }
      labels(first: 20) { nodes { name } }
      children(first: 50) { nodes { id identifier title url state { id name type position color } } }
    }
    pageInfo { hasNextPage endCursor }
  }
}";

pub const ISSUE_TEAM_STATES_QUERY: &str = "query NodalIssueTeamStates($id: String!, $stateId: String!) {
  issue(id: $id) { team { states(first: 100) { nodes { id name type position color } } } }
  workflowState(id: $stateId) { id name type position color }
}";

pub const SET_STATE_MUTATION: &str = "mutation NodalSetState($id: String!, $stateId: String!) {
  issueUpdate(id: $id, input: { stateId: $stateId }) { success issue { state { id name type position color } } }
}";

pub const COMMENT_MUTATION: &str = "mutation NodalComment($issueId: String!, $body: String!) {
  commentCreate(input: { issueId: $issueId, body: $body }) { success }
}";

/// Issue comments whose body contains `marker` (outbox idempotency).
pub const COMMENT_MARKER_QUERY: &str = "query NodalCommentMarker($id: String!, $marker: String!) {
  issue(id: $id) { comments(first: 1, filter: { body: { contains: $marker } }) { nodes { id } } }
}";

/// Importables listing filter: scope (`team` or `project`), state types, text (title, or
/// number if it looks like an identifier `ENG-12` / `12`) and created after a date.
pub fn importable_filter(
    scope_kind: &str,
    scope_id: &str,
    state_types: &[&str],
    text: Option<&str>,
    created_after_iso: Option<&str>,
) -> Value {
    let mut and = vec![match scope_kind {
        "project" => json!({ "project": { "id": { "eq": scope_id } } }),
        _ => json!({ "team": { "id": { "eq": scope_id } } }),
    }];
    if !state_types.is_empty() {
        and.push(json!({ "state": { "type": { "in": state_types } } }));
    }
    if let Some(t) = text.map(str::trim).filter(|t| !t.is_empty()) {
        let mut or = vec![json!({ "title": { "containsIgnoreCase": t } })];
        let digits = t.rsplit_once('-').map(|(_, n)| n).unwrap_or(t);
        if let Ok(n) = digits.parse::<u32>() {
            or.push(json!({ "number": { "eq": n } }));
        }
        and.push(json!({ "or": or }));
    }
    if let Some(iso) = created_after_iso {
        and.push(json!({ "createdAt": { "gt": iso } }));
    }
    json!({ "and": and })
}

/// Project rule filter: link scope (`team` or `project`) + the rule's project, open
/// (`state_types`) and, with `closed_within_days`, also those completed or canceled less
/// than that many days ago (`completedAt`/`canceledAt` with a relative ISO 8601 duration,
/// like `issue_filter`); with `created_after_iso`, only those created after it.
pub fn project_rule_filter(
    scope_kind: &str,
    scope_id: &str,
    project_id: &str,
    state_types: &[&str],
    closed_within_days: Option<u32>,
    created_after_iso: Option<&str>,
) -> Value {
    let mut and = vec![match scope_kind {
        "project" => json!({ "project": { "id": { "eq": scope_id } } }),
        _ => json!({ "team": { "id": { "eq": scope_id } } }),
    }];
    and.push(json!({ "project": { "id": { "eq": project_id } } }));
    let open = (!state_types.is_empty()).then(|| json!({ "state": { "type": { "in": state_types } } }));
    match (open, closed_within_days) {
        (open, Some(days)) => {
            let since = format!("-P{days}D");
            let mut or: Vec<Value> = open.into_iter().collect();
            or.push(json!({ "completedAt": { "gt": since } }));
            or.push(json!({ "canceledAt": { "gt": since } }));
            and.push(json!({ "or": or }));
        }
        (Some(open), None) => and.push(open),
        (None, None) => {}
    }
    if let Some(iso) = created_after_iso {
        and.push(json!({ "createdAt": { "gt": iso } }));
    }
    json!({ "and": and })
}

// ---- Response interpretation ----

#[derive(Debug, Deserialize)]
struct GqlResponse<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Vec<GqlError>,
}

#[derive(Debug, Deserialize)]
struct GqlError {
    message: String,
    #[serde(default)]
    extensions: Option<GqlErrorExt>,
}

#[derive(Debug, Deserialize)]
struct GqlErrorExt {
    code: Option<String>,
    #[serde(rename = "userPresentableMessage")]
    user_presentable_message: Option<String>,
}

/// `extensions.code` of Linear's internal errors (retryable). Linear does not document the
/// full list: these are the generic GraphQL/Apollo codes for server failures.
const SERVER_CODES: &[&str] = &["INTERNAL_SERVER_ERROR", "INTERNAL_ERROR", "SERVICE_UNAVAILABLE", "TIMEOUT"];

/// Classifies by `extensions.code`, never by the message text.
fn classify(errors: &[GqlError]) -> LinearError {
    let mut server = false;
    for e in errors {
        let code = e.extensions.as_ref().and_then(|x| x.code.as_deref());
        match code {
            Some("AUTHENTICATION_ERROR") => return LinearError::InvalidKey,
            Some("RATELIMITED") => return LinearError::RateLimited,
            Some(c) if SERVER_CODES.contains(&c) => server = true,
            _ => {}
        }
    }
    let first = &errors[0];
    let msg = first
        .extensions
        .as_ref()
        .and_then(|x| x.user_presentable_message.clone())
        .unwrap_or_else(|| first.message.clone());
    if first.extensions.as_ref().and_then(|x| x.code.as_deref()) == Some("FORBIDDEN") {
        return LinearError::Api(format!("the API key lacks permission for this ({msg})"));
    }
    if server {
        return LinearError::Unavailable(msg);
    }
    LinearError::Api(msg)
}

/// Turns HTTP status + body into typed data or a readable error.
/// Linear returns auth errors as HTTP 400 with `extensions.code`, so the body is parsed
/// before looking at the status.
pub fn interpret_response<T: DeserializeOwned>(status: u16, body: &str) -> Result<T, LinearError> {
    let parsed: Result<GqlResponse<T>, _> = serde_json::from_str(body);
    match parsed {
        Ok(resp) if !resp.errors.is_empty() => Err(classify(&resp.errors)),
        Ok(GqlResponse { data: Some(d), .. }) if (200..300).contains(&status) => Ok(d),
        _ => Err(match status {
            401 | 403 => LinearError::InvalidKey,
            429 => LinearError::RateLimited,
            s if s >= 500 || s == 408 => LinearError::Unavailable(format!("Linear is unavailable (HTTP {s})")),
            s if (200..300).contains(&s) => LinearError::Unavailable("unexpected response".into()),
            s => LinearError::Api(format!("HTTP {s}")),
        }),
    }
}

#[cfg(test)]
mod tests;
