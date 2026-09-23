//! Cliente HTTP mínimo para la API GraphQL de Linear.

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
        .user_agent(concat!("agent-desk/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("config de reqwest válida")
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
        teams.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
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
}
