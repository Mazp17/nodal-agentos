//! Fictitious GraphQL fixtures (workspace "acme"), shaped like the real responses.
use super::*;
use crate::model::{interpret_response, IssueUpdateData, SyncIssuesData, WorkflowStatesData};
use serde_json::json;

/// No tauri runtime here: a plain multi-thread tokio runtime for the ignored live test.
fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
        .block_on(f)
}

const ISSUES_BY_IDS: &str = r##"{"data":{"issues":{"nodes":[{
      "id":"uuid-142","identifier":"ENG-142","title":"Website rebrand",
      "url":"https://linear.app/acme/issue/ENG-142/website-rebrand",
      "priority":2,"updatedAt":"2026-09-20T10:00:00.000Z",
      "description":"Change the logo.\n\n## Acceptance criteria\n- New logo in the header\n- [ ] Favicon updated\n",
      "state":{"id":"st-prog","name":"In Progress","type":"started","position":3,"color":"#f2c94c"},
      "team":{"id":"team-eng","key":"ENG","name":"Engineering"},
      "project":{"id":"proj-web","name":"Website"},
      "assignee":{"id":"u1","name":"Ana Pérez","displayName":"ana"},
      "parent":{"id":"uuid-100","identifier":"ENG-100","title":"New brand","url":"https://linear.app/acme/issue/ENG-100/new-brand"},
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
    assert_eq!(
        item.scopes
            .iter()
            .map(|s| s.kind.as_str())
            .collect::<Vec<_>>(),
        vec!["team", "project"]
    );
    assert!(item.description_md.unwrap().contains("Acceptance"));
}

#[test]
fn list_page_without_optional_fields() {
    let body = r##"{"data":{"issues":{"nodes":[{
          "id":"uuid-7","identifier":"ENG-7","title":"Something","url":"https://linear.app/acme/issue/ENG-7/something",
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
    multi[1].team = crate::model::Team {
        id: "t2".into(),
        key: "OPS".into(),
        name: "Ops".into(),
    };
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
    assert_eq!(
        ext_state(&d.issue_update.issue.unwrap().state).name,
        "In Review"
    );
    let d: crate::model::CommentCreateData =
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
    let d: crate::model::IssueTeamStatesData = interpret_response(200, body).unwrap();
    let team = d.issue.team.states.nodes;
    assert_eq!(
        pick_team_state(&team, &d.workflow_state).unwrap().id,
        "ops-rev"
    );
    assert_eq!(pick_team_state(&team, &team[0]).unwrap().id, "ops-prog");
    let mut other = d.workflow_state.clone();
    other.name = "Blocked".into();
    assert!(pick_team_state(&team, &other).is_none());
}

#[test]
fn comment_marker_lookup_parses() {
    let hit: crate::model::CommentMarkerData = interpret_response(
        200,
        r#"{"data":{"issue":{"comments":{"nodes":[{"id":"c1"}]}}}}"#,
    )
    .unwrap();
    assert_eq!(hit.issue.unwrap().comments.nodes.len(), 1);
    let gone: crate::model::CommentMarkerData =
        interpret_response(200, r#"{"data":{"issue":null}}"#).unwrap();
    assert!(gone.issue.is_none());
}

#[test]
fn importable_filter_shape() {
    let f = importable_filter(
        "team",
        "team-eng",
        &["started", "unstarted"],
        Some(" ENG-12 "),
        Some("2026-01-01T00:00:00.000Z"),
    );
    assert_eq!(f["and"][0], json!({"team": {"id": {"eq": "team-eng"}}}));
    assert_eq!(
        f["and"][1],
        json!({"state": {"type": {"in": ["started", "unstarted"]}}})
    );
    assert_eq!(
        f["and"][2]["or"][0],
        json!({"title": {"containsIgnoreCase": "ENG-12"}})
    );
    assert_eq!(f["and"][2]["or"][1], json!({"number": {"eq": 12}}));
    assert_eq!(
        f["and"][3],
        json!({"createdAt": {"gt": "2026-01-01T00:00:00.000Z"}})
    );

    let p = importable_filter("project", "proj-web", &[], Some("logo"), None);
    assert_eq!(p["and"][0], json!({"project": {"id": {"eq": "proj-web"}}}));
    assert_eq!(p["and"][1]["or"].as_array().unwrap().len(), 1);
    assert_eq!(p["and"].as_array().unwrap().len(), 2);
}

#[test]
fn project_rule_filter_shape() {
    // Backfill: team + project + (open | closed within 14 days).
    let f = project_rule_filter(
        "team",
        "team-eng",
        "proj-web",
        &["triage", "backlog", "unstarted", "started"],
        Some(14),
        None,
    );
    assert_eq!(f["and"][0], json!({"team": {"id": {"eq": "team-eng"}}}));
    assert_eq!(f["and"][1], json!({"project": {"id": {"eq": "proj-web"}}}));
    assert_eq!(
        f["and"][2],
        json!({"or": [
            {"state": {"type": {"in": ["triage", "backlog", "unstarted", "started"]}}},
            {"completedAt": {"gt": "-P14D"}},
            {"canceledAt": {"gt": "-P14D"}}
        ]})
    );
    assert_eq!(f["and"].as_array().unwrap().len(), 3);
    // Auto-import: open ones created after the rule.
    let a = project_rule_filter(
        "team",
        "team-eng",
        "proj-web",
        &["started"],
        None,
        Some("2026-09-10T00:00:00.000Z"),
    );
    assert_eq!(a["and"][2], json!({"state": {"type": {"in": ["started"]}}}));
    assert_eq!(
        a["and"][3],
        json!({"createdAt": {"gt": "2026-09-10T00:00:00.000Z"}})
    );
}

/// Backfill page (fictitious fixture): open, completed 2 days ago and canceled 30 days ago
/// (Linear should not return it; the local check drops it anyway).
#[test]
fn backfill_page_parses_dates_and_project() {
    let body = r##"{"data":{"issues":{"nodes":[
          {"id":"u1","identifier":"ENG-1","title":"Open","url":"https://linear.app/acme/issue/ENG-1/a",
           "priority":0,"updatedAt":"2026-09-20T10:00:00.000Z","createdAt":"2026-08-01T10:00:00.000Z",
           "completedAt":null,"canceledAt":null,
           "state":{"id":"st-todo","name":"Todo","type":"unstarted","position":2,"color":"#e2e2e2"},
           "team":{"id":"team-eng","key":"ENG","name":"Engineering"},"project":{"id":"proj-web","name":"Website"},
           "labels":{"nodes":[]}},
          {"id":"u2","identifier":"ENG-2","title":"Done","url":"https://linear.app/acme/issue/ENG-2/b",
           "priority":3,"updatedAt":"2026-09-19T10:00:00.000Z","createdAt":"2026-08-02T10:00:00.000Z",
           "completedAt":"2026-09-19T10:00:00.000Z","canceledAt":null,
           "state":{"id":"st-done","name":"Done","type":"completed","position":6,"color":"#5e6ad2"},
           "team":{"id":"team-eng","key":"ENG","name":"Engineering"},"project":{"id":"proj-web","name":"Website"},
           "labels":{"nodes":[{"name":"frontend"}]}},
          {"id":"u3","identifier":"ENG-3","title":"Old","url":"https://linear.app/acme/issue/ENG-3/c",
           "priority":0,"updatedAt":"2026-08-22T10:00:00.000Z","createdAt":"2026-08-03T10:00:00.000Z",
           "completedAt":null,"canceledAt":"2026-08-22T10:00:00.000Z",
           "state":{"id":"st-canc","name":"Canceled","type":"canceled","position":7,"color":"#95a2b3"},
           "team":{"id":"team-eng","key":"ENG","name":"Engineering"},"project":{"id":"proj-web","name":"Website"},
           "labels":{"nodes":[]}}
        ],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}"##;
    let d: SyncIssuesData = interpret_response(200, body).unwrap();
    let items: Vec<_> = d.issues.nodes.into_iter().map(to_item).collect();
    assert_eq!(items[0].project().unwrap().name, "Website");
    assert_eq!(
        items[0].created_at.as_deref(),
        Some("2026-08-01T10:00:00.000Z")
    );
    assert_eq!(
        items[1].closed_at.as_deref(),
        Some("2026-09-19T10:00:00.000Z")
    );
    assert_eq!(
        items[2].closed_at.as_deref(),
        Some("2026-08-22T10:00:00.000Z")
    );
    let now = 1_789_948_800_000; // 2026-09-21
    let kept: Vec<_> = items
        .iter()
        .filter(|i| nodal_domain::sources::routing::backfill_keeps(i, now))
        .map(|i| i.identifier.as_str())
        .collect();
    assert_eq!(kept, vec!["ENG-1", "ENG-2"]);
}

#[test]
fn kinds_round_trip() {
    for k in [
        ExtKind::Triage,
        ExtKind::Backlog,
        ExtKind::Unstarted,
        ExtKind::Started,
        ExtKind::Completed,
        ExtKind::Canceled,
    ] {
        assert_eq!(ext_kind(linear_type(k).unwrap()), k);
    }
    assert_eq!(linear_type(ExtKind::Unknown), None);
    assert_eq!(ext_kind("weird"), ExtKind::Unknown);
}

/// Against the real API: `cargo test -- --ignored live_provider` with LINEAR_API_KEY.
/// Read-only: does not change states or comment.
#[test]
#[ignore]
fn live_provider_read_only() {
    let Some(key) = std::env::var("LINEAR_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
    else {
        eprintln!("LINEAR_API_KEY not set: skipping live test");
        return;
    };
    block_on(async {
        let p = LinearProvider::new(crate::client::http_client(), key.trim().to_string());
        println!("viewer: {}", p.status().await.expect("status"));
        let scopes = p.scopes().await.expect("scopes");
        println!("{} scopes", scopes.len());
        let Some(team) = scopes.iter().find(|s| s.kind == "team") else {
            return;
        };
        let states = p.states(team).await.expect("states");
        println!("{} states in {}", states.len(), team.name);
        let q = ImportQuery {
            scope: team.clone(),
            text: None,
            state_kinds: nodal_domain::model::providers::OPEN_KINDS.to_vec(),
            created_after: None,
            project_id: None,
            closed_within_days: None,
            cursor: None,
        };
        let page = p.list_importable(&q).await.expect("list");
        println!(
            "{} importable, more={}",
            page.items.len(),
            page.next_cursor.is_some()
        );
        if let Some(first) = page.items.first() {
            let full = p
                .fetch(&first.external_id)
                .await
                .expect("fetch")
                .expect("exists");
            println!(
                "{}: {} children, desc={}",
                full.identifier,
                full.children.len(),
                full.description_md.is_some()
            );
        }
    });
}
