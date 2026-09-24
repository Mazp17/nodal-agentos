//! Cliente HTTP mínimo para la API GraphQL de Linear.

use super::detail::{issue_detail_query, IssueDetail, IssueDetailData};
use super::error::LinearError;
use super::model::*;
use reqwest::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::time::Duration;

const ENDPOINT: &str = "https://api.linear.app/graphql";

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .user_agent(concat!("nodal/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("valid reqwest config")
}

pub struct LinearClient<'a> {
    http: &'a reqwest::Client,
    key: &'a str,
}

impl<'a> LinearClient<'a> {
    pub fn new(http: &'a reqwest::Client, key: &'a str) -> Self {
        Self { http, key }
    }

    async fn query<T: DeserializeOwned>(&self, query: &str, variables: Value) -> Result<T, LinearError> {
        // Personal API keys van sin "Bearer" (sólo OAuth lo usa):
        // https://linear.app/developers/graphql#personal-api-keys
        let mut auth = HeaderValue::from_str(self.key).map_err(|_| LinearError::InvalidKey)?;
        auth.set_sensitive(true);
        let resp = self
            .http
            .post(ENDPOINT)
            .header(AUTHORIZATION, auth)
            .header(CONTENT_TYPE, "application/json")
            .json(&json!({ "query": query, "variables": variables }))
            .send()
            .await?;
        let status = resp.status().as_u16();
        let body = resp.text().await?;
        interpret_response(status, &body)
    }

    pub async fn viewer(&self) -> Result<Viewer, LinearError> {
        let d: ViewerData = self.query(VIEWER_QUERY, json!({})).await?;
        Ok(d.viewer)
    }

    pub async fn teams(&self) -> Result<Vec<Team>, LinearError> {
        let d: TeamsData = self.query(TEAMS_QUERY, json!({})).await?;
        let mut teams = d.teams.nodes;
        teams.sort_by_key(|t| t.name.to_lowercase());
        Ok(teams)
    }

    pub async fn board(&self, team_ids: Option<&[String]>) -> Result<Board, LinearError> {
        let states: TeamStatesData = self
            .query(TEAM_STATES_QUERY, json!({ "filter": team_filter(team_ids) }))
            .await?;

        let filter = issue_filter(team_ids, RECENT_DONE_DAYS);
        let mut issues = Vec::new();
        let mut after: Option<String> = None;
        let mut truncated = false;
        for page in 0..MAX_PAGES {
            let d: IssuesData = self
                .query(
                    ISSUES_QUERY,
                    json!({ "filter": filter, "first": PAGE_SIZE, "after": after }),
                )
                .await?;
            issues.extend(d.issues.nodes);
            match (d.issues.page_info.has_next_page, d.issues.page_info.end_cursor) {
                (true, Some(cursor)) => {
                    after = Some(cursor);
                    truncated = page + 1 == MAX_PAGES;
                }
                _ => break,
            }
        }

        Ok(Board { teams: states.into_team_states(), issues, truncated })
    }

    /// Proyectos activos (ni completed ni canceled), por nombre.
    pub async fn projects(&self) -> Result<Vec<ProjectRef>, LinearError> {
        let d: ProjectsData = self.query(PROJECTS_QUERY, json!({})).await?;
        let mut out = d.projects.nodes;
        out.sort_by_key(|p| p.name.to_lowercase());
        Ok(out)
    }

    pub async fn project_teams(&self, project_id: &str) -> Result<Vec<Team>, LinearError> {
        let d: ProjectTeamsData = self.query(PROJECT_TEAMS_QUERY, json!({ "id": project_id })).await?;
        Ok(d.project.teams.nodes)
    }

    /// Estados (no archivados) de esos teams, ordenados por team y posición.
    pub async fn workflow_states(&self, team_ids: &[String]) -> Result<Vec<ScopedState>, LinearError> {
        let d: WorkflowStatesData = self.query(WORKFLOW_STATES_QUERY, json!({ "teamIds": team_ids })).await?;
        let mut out = d.workflow_states.nodes;
        out.sort_by(|a, b| a.team.key.cmp(&b.team.key).then(a.state.position.total_cmp(&b.state.position)));
        Ok(out)
    }

    /// Una página del listado de importables (`filter` de `model::importable_filter`).
    pub async fn importable_page(
        &self,
        filter: &Value,
        after: Option<&str>,
    ) -> Result<(Vec<SyncIssue>, Option<String>), LinearError> {
        let d: SyncIssuesData = self
            .query(IMPORTABLE_QUERY, json!({ "filter": filter, "first": SYNC_PAGE_SIZE, "after": after }))
            .await?;
        let next = match d.issues.page_info {
            PageInfo { has_next_page: true, end_cursor } => end_cursor,
            _ => None,
        };
        Ok((d.issues.nodes, next))
    }

    /// Issues completas por UUID, en lotes de `PULL_BATCH`. Las que no existen (o la key no
    /// ve) simplemente no vuelven.
    pub async fn issues_by_ids(&self, ids: &[String]) -> Result<Vec<SyncIssue>, LinearError> {
        let mut out = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(PULL_BATCH) {
            let d: SyncIssuesData = self
                .query(ISSUES_BY_IDS_QUERY, json!({ "ids": chunk, "first": chunk.len() }))
                .await?;
            out.extend(d.issues.nodes);
        }
        Ok(out)
    }

    /// Estados del team de la issue y el estado `state_id` (de cualquier team).
    pub async fn issue_team_states(
        &self,
        issue_id: &str,
        state_id: &str,
    ) -> Result<(Vec<WorkflowState>, WorkflowState), LinearError> {
        let d: IssueTeamStatesData = self
            .query(ISSUE_TEAM_STATES_QUERY, json!({ "id": issue_id, "stateId": state_id }))
            .await?;
        Ok((d.issue.team.states.nodes, d.workflow_state))
    }

    /// Cambia el estado y devuelve el estado resultante.
    pub async fn set_issue_state(&self, issue_id: &str, state_id: &str) -> Result<WorkflowState, LinearError> {
        let d: IssueUpdateData = self
            .query(SET_STATE_MUTATION, json!({ "id": issue_id, "stateId": state_id }))
            .await?;
        match d.issue_update {
            IssueUpdatePayload { success: true, issue: Some(i) } => Ok(i.state),
            _ => Err(LinearError::Api("issueUpdate returned success: false".into())),
        }
    }

    pub async fn create_comment(&self, issue_id: &str, body: &str) -> Result<(), LinearError> {
        let d: CommentCreateData = self
            .query(COMMENT_MUTATION, json!({ "issueId": issue_id, "body": body }))
            .await?;
        if d.comment_create.success {
            Ok(())
        } else {
            Err(LinearError::Api("commentCreate returned success: false".into()))
        }
    }

    /// Si la issue ya tiene un comentario que contiene `marker`.
    pub async fn has_comment_with(&self, issue_id: &str, marker: &str) -> Result<bool, LinearError> {
        let d: CommentMarkerData = self
            .query(COMMENT_MARKER_QUERY, json!({ "id": issue_id, "marker": marker }))
            .await?;
        Ok(d.issue.is_some_and(|i| !i.comments.nodes.is_empty()))
    }

    pub async fn issue_detail(&self, issue_id: &str) -> Result<IssueDetail, LinearError> {
        let d: IssueDetailData = self
            .query(&issue_detail_query(), json!({ "id": issue_id }))
            .await?;
        Ok(d.into_detail())
    }
}
