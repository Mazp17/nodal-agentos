//! Tipos, queries y parseo de respuestas GraphQL de Linear. Sin red: todo acá es puro
//! para poder testearlo con fixtures.

use super::error::LinearError;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Issues completed/canceled más viejas que esto no se traen al board.
pub const RECENT_DONE_DAYS: u32 = 14;
pub const PAGE_SIZE: u32 = 100;
/// Tope de páginas por board (100 × 20 = 2000 issues) para no colgar la UI si un
/// workspace es enorme; si se alcanza, el board sale con `truncated: true`.
pub const MAX_PAGES: usize = 20;

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParentRef {
    pub identifier: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub url: String,
    /// 0 = sin prioridad, 1 = urgente … 4 = baja.
    pub priority: f64,
    pub priority_label: String,
    pub team: Team,
    pub state: WorkflowState,
    pub assignee: Option<UserRef>,
    pub project: Option<ProjectRef>,
    pub parent: Option<ParentRef>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamStates {
    pub team: Team,
    pub states: Vec<WorkflowState>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Board {
    pub teams: Vec<TeamStates>,
    pub issues: Vec<Issue>,
    /// true si se cortó la paginación en MAX_PAGES.
    pub truncated: bool,
}

// ---- Formas crudas de la respuesta GraphQL ----

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
struct RawTeamStates {
    id: String,
    key: String,
    name: String,
    states: Connection<WorkflowState>,
}

#[derive(Debug, Deserialize)]
pub struct TeamStatesData {
    teams: Connection<RawTeamStates>,
}

impl TeamStatesData {
    pub fn into_team_states(self) -> Vec<TeamStates> {
        let mut out: Vec<TeamStates> = self
            .teams
            .nodes
            .into_iter()
            .map(|t| {
                let mut states = t.states.nodes;
                states.sort_by(|a, b| a.position.total_cmp(&b.position));
                TeamStates {
                    team: Team { id: t.id, key: t.key, name: t.name },
                    states,
                }
            })
            .collect();
        out.sort_by_key(|t| t.team.name.to_lowercase());
        out
    }
}

#[derive(Debug, Deserialize)]
pub struct IssuesData {
    pub issues: PagedConnection<Issue>,
}

// ---- Queries ----

pub const VIEWER_QUERY: &str = "query Viewer { viewer { name email } }";

pub const TEAMS_QUERY: &str = "query Teams { teams(first: 250) { nodes { id key name } } }";

/// Tamaños acotados a propósito: Linear puntúa la complejidad multiplicando por `first`
/// en conexiones anidadas (límite 10k por query); 50×50 ≈ 3.8k.
pub const TEAM_STATES_QUERY: &str = "query TeamStates($filter: TeamFilter) {
  teams(first: 50, filter: $filter) {
    nodes { id key name states(first: 50) { nodes { id name type position color } } }
  }
}";

pub const ISSUES_QUERY: &str = "query BoardIssues($filter: IssueFilter, $first: Int!, $after: String) {
  issues(filter: $filter, first: $first, after: $after, orderBy: updatedAt) {
    nodes {
      id identifier title url priority priorityLabel updatedAt
      team { id key name }
      state { id name type position color }
      assignee { id name displayName }
      project { id name }
      parent { identifier }
    }
    pageInfo { hasNextPage endCursor }
  }
}";

/// Lista de team ids efectiva: `None` o vacía = todos los teams.
fn non_empty(team_ids: Option<&[String]>) -> Option<&[String]> {
    team_ids.filter(|ids| !ids.is_empty())
}

/// Filtro de issues: abiertas (cualquier estado que no sea completed/canceled) o
/// cerradas (completedAt/canceledAt, no updatedAt: un comentario en una issue vieja no
/// la trae de vuelta) hace menos de `recent_days`. Linear acepta duraciones ISO 8601
/// relativas ("-P14D") en comparadores de fecha, así no dependemos del reloj local.
pub fn issue_filter(team_ids: Option<&[String]>, recent_days: u32) -> Value {
    let since = format!("-P{recent_days}D");
    let open_or_recent = json!({ "or": [
        { "state": { "type": { "nin": ["completed", "canceled"] } } },
        { "completedAt": { "gt": since } },
        { "canceledAt": { "gt": since } }
    ]});
    match non_empty(team_ids) {
        Some(ids) => json!({ "and": [{ "team": { "id": { "in": ids } } }, open_or_recent] }),
        None => open_or_recent,
    }
}

pub fn team_filter(team_ids: Option<&[String]>) -> Value {
    match non_empty(team_ids) {
        Some(ids) => json!({ "id": { "in": ids } }),
        None => Value::Null,
    }
}

// ---- Interpretación de respuestas ----

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

fn classify(errors: &[GqlError]) -> LinearError {
    for e in errors {
        let code = e.extensions.as_ref().and_then(|x| x.code.as_deref());
        match code {
            Some("AUTHENTICATION_ERROR") => return LinearError::InvalidKey,
            Some("RATELIMITED") => return LinearError::RateLimited,
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
    LinearError::Api(msg)
}

/// Convierte status HTTP + cuerpo en datos tipados o en un error legible.
/// Linear responde errores de auth como HTTP 400 con `extensions.code`, por eso se
/// intenta parsear el cuerpo antes de mirar el status.
pub fn interpret_response<T: DeserializeOwned>(status: u16, body: &str) -> Result<T, LinearError> {
    let parsed: Result<GqlResponse<T>, _> = serde_json::from_str(body);
    match parsed {
        Ok(resp) if !resp.errors.is_empty() => Err(classify(&resp.errors)),
        Ok(GqlResponse { data: Some(d), .. }) if (200..300).contains(&status) => Ok(d),
        _ => Err(match status {
            401 | 403 => LinearError::InvalidKey,
            429 => LinearError::RateLimited,
            s if s >= 500 => LinearError::Api(format!("Linear is unavailable (HTTP {s})")),
            s if (200..300).contains(&s) => LinearError::Api("unexpected response".into()),
            s => LinearError::Api(format!("HTTP {s}")),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ISSUES_PAGE: &str = r##"{
      "data": { "issues": {
        "nodes": [
          {
            "id": "i1", "identifier": "ACME-8", "title": "Arreglar login",
            "url": "https://linear.app/acme/issue/ACME-8/arreglar-login",
            "priority": 2, "priorityLabel": "High", "updatedAt": "2026-09-20T10:00:00.000Z",
            "team": { "id": "t1", "key": "ACME", "name": "Acme" },
            "state": { "id": "s3", "name": "In Review", "type": "started", "position": 3, "color": "#0f783c" },
            "assignee": { "id": "u1", "name": "Jane Doe", "displayName": "jane" },
            "project": { "id": "p1", "name": "Auth" },
            "parent": { "identifier": "ACME-2" }
          },
          {
            "id": "i2", "identifier": "OPS-1", "title": "Sin asignar",
            "url": "https://linear.app/acme/issue/OPS-1/sin-asignar",
            "priority": 0, "priorityLabel": "No priority", "updatedAt": "2026-09-21T10:00:00.000Z",
            "team": { "id": "t2", "key": "OPS", "name": "Ops" },
            "state": { "id": "s9", "name": "Backlog", "type": "backlog", "position": 0.5, "color": "#bec2c8" },
            "assignee": null, "project": null, "parent": null
          }
        ],
        "pageInfo": { "hasNextPage": true, "endCursor": "abc" }
      } }
    }"##;

    #[test]
    fn parses_issue_page_with_optional_fields() {
        let d: IssuesData = interpret_response(200, ISSUES_PAGE).unwrap();
        assert_eq!(d.issues.nodes.len(), 2);
        assert!(d.issues.page_info.has_next_page);
        assert_eq!(d.issues.page_info.end_cursor.as_deref(), Some("abc"));

        let a = &d.issues.nodes[0];
        assert_eq!(a.identifier, "ACME-8");
        assert_eq!(a.state.state_type, "started");
        assert_eq!(a.assignee.as_ref().unwrap().display_name, "jane");
        assert_eq!(a.parent.as_ref().unwrap().identifier, "ACME-2");
        assert_eq!(a.project.as_ref().unwrap().id, "p1");

        let b = &d.issues.nodes[1];
        assert!(b.assignee.is_none() && b.project.is_none() && b.parent.is_none());
        assert_eq!(b.state.position, 0.5);
    }

    #[test]
    fn serializes_issue_camel_case_for_frontend() {
        let d: IssuesData = interpret_response(200, ISSUES_PAGE).unwrap();
        let v = serde_json::to_value(&d.issues.nodes[0]).unwrap();
        assert_eq!(v["priorityLabel"], "High");
        assert_eq!(v["updatedAt"], "2026-09-20T10:00:00.000Z");
        assert_eq!(v["state"]["type"], "started");
        assert_eq!(v["assignee"]["displayName"], "jane");
    }

    #[test]
    fn parses_viewer_and_teams() {
        let v: ViewerData =
            interpret_response(200, r#"{"data":{"viewer":{"name":"Ana","email":"a@x.com"}}}"#)
                .unwrap();
        assert_eq!(v.viewer, Viewer { name: "Ana".into(), email: "a@x.com".into() });

        let t: TeamsData = interpret_response(
            200,
            r#"{"data":{"teams":{"nodes":[{"id":"t1","key":"ACME","name":"Acme"}]}}}"#,
        )
        .unwrap();
        assert_eq!(t.teams.nodes[0].key, "ACME");
    }

    #[test]
    fn team_states_sorted_by_position_and_teams_by_name() {
        let body = r##"{"data":{"teams":{"nodes":[
          {"id":"t2","key":"OPS","name":"ops","states":{"nodes":[]}},
          {"id":"t1","key":"ACME","name":"Acme","states":{"nodes":[
            {"id":"b","name":"Done","type":"completed","position":4,"color":"#000"},
            {"id":"a","name":"Todo","type":"unstarted","position":1,"color":"#fff"}
          ]}}
        ]}}}"##;
        let d: TeamStatesData = interpret_response(200, body).unwrap();
        let ts = d.into_team_states();
        assert_eq!(ts[0].team.key, "ACME");
        assert_eq!(ts[1].team.key, "OPS");
        assert_eq!(ts[0].states[0].name, "Todo");
    }

    #[test]
    fn auth_error_maps_to_invalid_key() {
        let body = r#"{"errors":[{"message":"Authentication required, not authenticated",
          "extensions":{"type":"authentication error","code":"AUTHENTICATION_ERROR",
          "userPresentableMessage":"You need to authenticate."}}]}"#;
        let e = interpret_response::<ViewerData>(400, body).unwrap_err();
        assert_eq!(e, LinearError::InvalidKey);
        assert_eq!(e.kind(), "invalidKey");
    }

    #[test]
    fn rate_limit_and_generic_errors() {
        let rl = r#"{"errors":[{"message":"Rate limit exceeded","extensions":{"code":"RATELIMITED"}}]}"#;
        assert_eq!(interpret_response::<ViewerData>(400, rl).unwrap_err(), LinearError::RateLimited);
        assert_eq!(interpret_response::<ViewerData>(429, "").unwrap_err(), LinearError::RateLimited);

        let other = r#"{"data":null,"errors":[{"message":"Argument Validation Error",
          "extensions":{"code":"INVALID_INPUT","userPresentableMessage":"Invalid filter"}}]}"#;
        assert_eq!(
            interpret_response::<ViewerData>(400, other).unwrap_err(),
            LinearError::Api("Invalid filter".into())
        );
        assert_eq!(interpret_response::<ViewerData>(401, "no json").unwrap_err(), LinearError::InvalidKey);
        assert!(matches!(
            interpret_response::<ViewerData>(502, "<html>").unwrap_err(),
            LinearError::Api(m) if m.contains("502")
        ));
        assert!(matches!(
            interpret_response::<ViewerData>(200, r#"{"data":{}}"#).unwrap_err(),
            LinearError::Api(_)
        ));
    }

    #[test]
    fn error_serializes_as_kind_and_message() {
        let v = serde_json::to_value(LinearError::MissingKey).unwrap();
        assert_eq!(v["kind"], "missingKey");
        assert!(v["message"].as_str().unwrap().contains("API key"));
    }

    #[test]
    fn issue_filter_with_and_without_teams() {
        let all = issue_filter(None, 14);
        assert!(all.get("and").is_none());
        assert_eq!(all["or"][0]["state"]["type"]["nin"], json!(["completed", "canceled"]));
        assert_eq!(all["or"][1]["completedAt"]["gt"], "-P14D");
        assert_eq!(all["or"][2]["canceledAt"]["gt"], "-P14D");

        assert_eq!(issue_filter(Some(&[]), 14), all, "empty list = all teams");

        let ids = vec!["t1".to_string(), "t2".to_string()];
        let some = issue_filter(Some(&ids), 14);
        assert_eq!(some["and"][0]["team"]["id"]["in"], json!(["t1", "t2"]));
        assert_eq!(some["and"][1], all);

        assert_eq!(team_filter(None), Value::Null);
        assert_eq!(team_filter(Some(&ids))["id"]["in"], json!(["t1", "t2"]));
    }
}
