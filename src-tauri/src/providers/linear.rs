//! Adapter de Linear. Las queries y el parseo viven en `crate::linear::{client,model}`;
//! acá solo se traduce al modelo genérico. Scopes: `team` y `project`.

use crate::domain::{ExtKind, ExternalState, Priority, ScopeRef};
use crate::linear::client::LinearClient;
use crate::linear::model::{importable_filter, pick_team_state, ScopedState, SyncIssue, WorkflowState};

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

/// `WorkflowState.type` → tipo normalizado.
pub fn ext_kind(state_type: &str) -> ExtKind {
    match state_type {
        "triage" => ExtKind::Triage,
        "backlog" => ExtKind::Backlog,
        "unstarted" => ExtKind::Unstarted,
        "started" => ExtKind::Started,
        "completed" => ExtKind::Completed,
        // Por si Linear expone "duplicate" como tipo propio (hoy es `canceled`).
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

/// 0 = sin prioridad, 1 = urgente … 4 = baja.
pub fn priority(p: f64) -> Priority {
    match p.round() as i64 {
        1 => Priority::Urgent,
        2 => Priority::High,
        3 => Priority::Medium,
        4 => Priority::Low,
        _ => Priority::None,
    }
}

/// Estados de uno o varios teams. Con varios (proyecto multi-team) el nombre lleva el key del
/// team para distinguir "ENG · In Progress" de "OPS · In Progress".
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

    async fn list_importable(&self, q: &ImportQuery) -> ProviderResult<Page> {
        let types: Vec<&str> = q.state_kinds.iter().filter_map(|k| linear_type(*k)).collect();
        let created = q.created_after.map(iso_from_ms);
        let filter = importable_filter(&q.scope.kind, &q.scope.id, &types, q.text.as_deref(), created.as_deref());
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
        // Un query más por push (son pocos): en un proyecto multi-team el mapeo apunta al
        // estado de un team, y la issue puede ser de otro.
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
}

#[cfg(test)]
mod tests {
    //! Fixtures GraphQL ficticios (workspace "acme"), con la forma de las respuestas reales.
    use super::*;
    use crate::linear::model::{interpret_response, IssueUpdateData, SyncIssuesData, WorkflowStatesData};
    use serde_json::json;

    const ISSUES_BY_IDS: &str = r##"{"data":{"issues":{"nodes":[{
      "id":"uuid-142","identifier":"ENG-142","title":"Rebrand de la web",
      "url":"https://linear.app/acme/issue/ENG-142/rebrand-de-la-web",
      "priority":2,"updatedAt":"2026-09-20T10:00:00.000Z",
      "description":"Cambiar el logo.\n\n## Acceptance criteria\n- Logo nuevo en el header\n- [ ] Favicon actualizado\n",
      "state":{"id":"st-prog","name":"In Progress","type":"started","position":3,"color":"#f2c94c"},
      "team":{"id":"team-eng","key":"ENG","name":"Engineering"},
      "project":{"id":"proj-web","name":"Website"},
      "assignee":{"id":"u1","name":"Ana Pérez","displayName":"ana"},
      "parent":{"id":"uuid-100","identifier":"ENG-100","title":"Marca nueva","url":"https://linear.app/acme/issue/ENG-100/marca-nueva"},
      "labels":{"nodes":[{"name":"frontend"},{"name":"brand"}]},
      "children":{"nodes":[
        {"id":"uuid-143","identifier":"ENG-143","title":"Header","url":"https://linear.app/acme/issue/ENG-143/header",
         "state":{"id":"st-done","name":"Done","type":"completed","position":6,"color":"#5e6ad2"}}
      ]}
    }],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}"##;

    #[test]
    fn full_issue_maps_to_external_item() {
        let d: SyncIssuesData = interpret_response(200, ISSUES_BY_IDS).unwrap();
        let item = to_item(d.issues.nodes.into_iter().next().unwrap());
        assert_eq!(item.external_id, "uuid-142");
        assert_eq!(item.identifier, "ENG-142");
        assert_eq!(item.priority, Priority::High);
        assert_eq!(item.state.kind, ExtKind::Started);
        assert_eq!(item.state.color.as_deref(), Some("#f2c94c"));
        assert_eq!(item.labels, vec!["frontend", "brand"]);
        assert_eq!(item.assignee.as_deref(), Some("Ana Pérez"));
        assert_eq!(item.parent.as_ref().unwrap().identifier, "ENG-100");
        assert_eq!(item.children.len(), 1);
        assert_eq!(item.children[0].state.kind, ExtKind::Completed);
        assert_eq!(item.scopes.iter().map(|s| s.kind.as_str()).collect::<Vec<_>>(), vec!["team", "project"]);
        assert!(item.description_md.unwrap().contains("Acceptance"));
    }

    #[test]
    fn list_page_without_optional_fields() {
        let body = r##"{"data":{"issues":{"nodes":[{
          "id":"uuid-7","identifier":"ENG-7","title":"Algo","url":"https://linear.app/acme/issue/ENG-7/algo",
          "priority":0,"updatedAt":"2026-09-20T10:00:00.000Z",
          "state":{"id":"st-todo","name":"Todo","type":"unstarted","position":2,"color":"#e2e2e2"},
          "team":{"id":"team-eng","key":"ENG","name":"Engineering"},"project":null,
          "labels":{"nodes":[]}
        }],"pageInfo":{"hasNextPage":true,"endCursor":"cur-1"}}}}"##;
        let d: SyncIssuesData = interpret_response(200, body).unwrap();
        assert_eq!(d.issues.page_info.end_cursor.as_deref(), Some("cur-1"));
        let item = to_item(d.issues.nodes.into_iter().next().unwrap());
        assert_eq!(item.priority, Priority::None);
        assert!(item.description_md.is_none() && item.parent.is_none() && item.children.is_empty());
        assert_eq!(item.scopes.len(), 1);
    }

    #[test]
    fn workflow_states_single_and_multi_team() {
        let body = r##"{"data":{"workflowStates":{"nodes":[
          {"id":"a","name":"Todo","type":"unstarted","position":1,"color":"#fff","team":{"id":"t1","key":"ENG","name":"Engineering"}},
          {"id":"b","name":"Duplicate","type":"canceled","position":9,"color":"#000","team":{"id":"t1","key":"ENG","name":"Engineering"}}
        ]}}}"##;
        let d: WorkflowStatesData = interpret_response(200, body).unwrap();
        let s = scoped_states(&d.workflow_states.nodes);
        assert_eq!(s[0].name, "Todo");
        assert_eq!(s[1].kind, ExtKind::Canceled);

        let mut multi = d.workflow_states.nodes.clone();
        multi[1].team = crate::linear::model::Team { id: "t2".into(), key: "OPS".into(), name: "Ops".into() };
        let s = scoped_states(&multi);
        assert_eq!(s[0].name, "ENG · Todo");
        assert_eq!(s[1].name, "OPS · Duplicate");
    }

    #[test]
    fn mutation_payloads_parse() {
        let d: IssueUpdateData = interpret_response(
            200,
            r##"{"data":{"issueUpdate":{"success":true,"issue":{"state":{"id":"s2","name":"In Review","type":"started","position":4,"color":"#0f783c"}}}}}"##,
        )
        .unwrap();
        assert!(d.issue_update.success);
        assert_eq!(ext_state(&d.issue_update.issue.unwrap().state).name, "In Review");
        let d: crate::linear::model::CommentCreateData =
            interpret_response(200, r#"{"data":{"commentCreate":{"success":false}}}"#).unwrap();
        assert!(!d.comment_create.success);
    }

    #[test]
    fn push_state_is_resolved_in_the_issue_team() {
        let body = r##"{"data":{
          "issue":{"team":{"states":{"nodes":[
            {"id":"ops-prog","name":"In Progress","type":"started","position":2,"color":"#f2c94c"},
            {"id":"ops-rev","name":"In review","type":"started","position":3,"color":"#0f783c"}
          ]}}},
          "workflowState":{"id":"eng-rev","name":"In Review","type":"started","position":3,"color":"#0f783c"}
        }}"##;
        let d: crate::linear::model::IssueTeamStatesData = interpret_response(200, body).unwrap();
        let team = d.issue.team.states.nodes;
        assert_eq!(pick_team_state(&team, &d.workflow_state).unwrap().id, "ops-rev");
        assert_eq!(pick_team_state(&team, &team[0]).unwrap().id, "ops-prog");
        let mut other = d.workflow_state.clone();
        other.name = "Blocked".into();
        assert!(pick_team_state(&team, &other).is_none());
    }

    #[test]
    fn importable_filter_shape() {
        let f = importable_filter("team", "team-eng", &["started", "unstarted"], Some(" ENG-12 "), Some("2026-01-01T00:00:00.000Z"));
        assert_eq!(f["and"][0], json!({"team": {"id": {"eq": "team-eng"}}}));
        assert_eq!(f["and"][1], json!({"state": {"type": {"in": ["started", "unstarted"]}}}));
        assert_eq!(f["and"][2]["or"][0], json!({"title": {"containsIgnoreCase": "ENG-12"}}));
        assert_eq!(f["and"][2]["or"][1], json!({"number": {"eq": 12}}));
        assert_eq!(f["and"][3], json!({"createdAt": {"gt": "2026-01-01T00:00:00.000Z"}}));

        let p = importable_filter("project", "proj-web", &[], Some("logo"), None);
        assert_eq!(p["and"][0], json!({"project": {"id": {"eq": "proj-web"}}}));
        assert_eq!(p["and"][1]["or"].as_array().unwrap().len(), 1);
        assert_eq!(p["and"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn kinds_round_trip() {
        for k in [ExtKind::Triage, ExtKind::Backlog, ExtKind::Unstarted, ExtKind::Started, ExtKind::Completed, ExtKind::Canceled] {
            assert_eq!(ext_kind(linear_type(k).unwrap()), k);
        }
        assert_eq!(linear_type(ExtKind::Unknown), None);
        assert_eq!(ext_kind("weird"), ExtKind::Unknown);
    }

    /// Contra la API real: `cargo test -- --ignored live_provider` con LINEAR_API_KEY.
    /// Solo lectura: no cambia estados ni comenta.
    #[test]
    #[ignore]
    fn live_provider_read_only() {
        let Some(key) = std::env::var("LINEAR_API_KEY").ok().filter(|k| !k.trim().is_empty()) else {
            eprintln!("LINEAR_API_KEY not set: skipping live test");
            return;
        };
        tauri::async_runtime::block_on(async {
            let p = LinearProvider::new(crate::linear::client::http_client(), key.trim().to_string());
            println!("viewer: {}", p.status().await.expect("status"));
            let scopes = p.scopes().await.expect("scopes");
            println!("{} scopes", scopes.len());
            let Some(team) = scopes.iter().find(|s| s.kind == "team") else { return };
            let states = p.states(team).await.expect("states");
            println!("{} states in {}", states.len(), team.name);
            let q = ImportQuery {
                scope: team.clone(),
                text: None,
                state_kinds: super::super::OPEN_KINDS.to_vec(),
                created_after: None,
                cursor: None,
            };
            let page = p.list_importable(&q).await.expect("list");
            println!("{} importable, more={}", page.items.len(), page.next_cursor.is_some());
            if let Some(first) = page.items.first() {
                let full = p.fetch(&first.external_id).await.expect("fetch").expect("exists");
                println!("{}: {} children, desc={}", full.identifier, full.children.len(), full.description_md.is_some());
            }
        });
    }
}
