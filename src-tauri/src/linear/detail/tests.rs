use super::*;
use crate::linear::model::interpret_response;
use crate::linear::LinearError;

const DETAIL: &str = r###"{ "data": { "issue": {
      "id": "i1", "identifier": "ACME-8",
      "description": "## Goal\n\nFix **login**.\n\n<script>alert(1)</script>",
      "estimate": 3, "dueDate": "2026-10-01", "createdAt": "2026-09-01T10:00:00.000Z",
      "creator": { "id": "u1", "name": "Jane Doe", "displayName": "jane" },
      "parent": { "id": "i0", "identifier": "ACME-2", "title": "Auth epic",
        "state": { "name": "In Progress", "type": "started", "color": "#f2c94c" } },
      "labels": { "nodes": [ { "id": "l1", "name": "Bug", "color": "#eb5757" } ] },
      "children": {
        "nodes": [
          { "id": "c1", "identifier": "ACME-9", "title": "Backend", "assignee": null,
            "state": { "name": "Done", "type": "completed", "color": "#5e6ad2" } },
          { "id": "c2", "identifier": "ACME-10", "title": "Frontend",
            "assignee": { "id": "u2", "name": "Ana Pérez", "displayName": "" },
            "state": { "name": "Todo", "type": "unstarted", "color": "#e2e2e2" } }
        ],
        "pageInfo": { "hasNextPage": true, "endCursor": "x" }
      },
      "relations": { "nodes": [
        { "type": "blocks", "relatedIssue": { "id": "r1", "identifier": "OPS-1", "title": "Deploy",
          "state": { "name": "Todo", "type": "unstarted", "color": "#e2e2e2" } } },
        { "type": "related", "relatedIssue": { "id": "r2", "identifier": "OPS-2", "title": "Docs",
          "state": { "name": "Backlog", "type": "backlog", "color": "#bec2c8" } } },
        { "type": "similar", "relatedIssue": { "id": "r9", "identifier": "OPS-9", "title": "Noise",
          "state": { "name": "Backlog", "type": "backlog", "color": "#bec2c8" } } },
        { "type": "duplicate", "relatedIssue": { "id": "r5", "identifier": "ACME-1", "title": "Old login bug",
          "state": { "name": "Canceled", "type": "canceled", "color": "#95a2b3" } } }
      ] },
      "inverseRelations": { "nodes": [
        { "type": "blocks", "issue": { "id": "r3", "identifier": "CORE-7", "title": "API ready",
          "state": { "name": "In Progress", "type": "started", "color": "#f2c94c" } } },
        { "type": "related", "issue": { "id": "r2", "identifier": "OPS-2", "title": "Docs",
          "state": { "name": "Backlog", "type": "backlog", "color": "#bec2c8" } } }
      ] },
      "comments": {
        "nodes": [
          { "id": "m2", "body": "Second", "createdAt": "2026-09-03T10:00:00.000Z",
            "user": null, "botActor": { "name": "GitHub" } },
          { "id": "m1", "body": "First", "createdAt": "2026-09-02T10:00:00.000Z",
            "user": { "id": "u1", "name": "Jane Doe", "displayName": "jane" }, "botActor": null },
          { "id": "m3", "body": "Third", "createdAt": "2026-09-04T10:00:00.000Z",
            "user": null, "botActor": null }
        ],
        "pageInfo": { "hasNextPage": false, "endCursor": null }
      }
    } } }"###;

fn parse(body: &str) -> IssueDetail {
    interpret_response::<IssueDetailData>(200, body).unwrap().into_detail()
}

#[test]
fn parses_scalar_fields_parent_and_labels() {
    let d = parse(DETAIL);
    assert_eq!(d.identifier, "ACME-8");
    assert!(d.description.as_deref().unwrap().starts_with("## Goal"));
    assert_eq!(d.estimate, Some(3.0));
    assert_eq!(d.due_date.as_deref(), Some("2026-10-01"));
    assert_eq!(d.creator.as_ref().unwrap().display_name, "jane");
    assert_eq!(d.parent.as_ref().unwrap().identifier, "ACME-2");
    assert_eq!(d.parent.as_ref().unwrap().state.state_type, "started");
    assert_eq!(d.labels, vec![Label { id: "l1".into(), name: "Bug".into(), color: "#eb5757".into() }]);
}

#[test]
fn parses_children_with_truncation() {
    let d = parse(DETAIL);
    assert_eq!(d.children.len(), 2);
    assert!(d.children_truncated);
    assert_eq!(d.children[0].state.state_type, "completed");
    assert!(d.children[0].assignee.is_none());
    assert_eq!(d.children[1].assignee.as_ref().unwrap().name, "Ana Pérez");
}

#[test]
fn maps_relations_by_direction_and_drops_similar_and_dupes() {
    let d = parse(DETAIL);
    let got: Vec<(RelationKind, bool, &str)> = d
        .relations
        .iter()
        .map(|r| (r.kind, r.inverse, r.issue.identifier.as_str()))
        .collect();
    assert_eq!(
        got,
        vec![
            (RelationKind::Blocks, false, "OPS-1"),
            (RelationKind::Related, false, "OPS-2"),
            (RelationKind::Duplicate, false, "ACME-1"),
            (RelationKind::BlockedBy, false, "CORE-7"),
        ]
    );
}

#[test]
fn comments_sorted_chronologically_with_author_fallbacks() {
    let d = parse(DETAIL);
    let got: Vec<(&str, Option<&str>)> =
        d.comments.iter().map(|c| (c.body.as_str(), c.author.as_deref())).collect();
    assert_eq!(
        got,
        vec![("First", Some("jane")), ("Second", Some("GitHub")), ("Third", None)]
    );
    assert!(!d.comments_truncated);
}

#[test]
fn empty_collections_and_blank_description() {
    let body = r#"{"data":{"issue":{
          "id":"i1","identifier":"A-1","description":"  \n","estimate":null,"dueDate":null,
          "createdAt":"2026-09-01T10:00:00.000Z","creator":null,"parent":null,
          "labels":{"nodes":[]},
          "children":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},
          "relations":{"nodes":[]},"inverseRelations":{"nodes":[]},
          "comments":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}
        }}}"#;
    let d = parse(body);
    assert!(d.description.is_none() && d.parent.is_none() && d.creator.is_none());
    assert!(d.children.is_empty() && d.relations.is_empty() && d.comments.is_empty());
}

#[test]
fn serializes_camel_case_and_snake_case_kinds() {
    let v = serde_json::to_value(parse(DETAIL)).unwrap();
    assert_eq!(v["dueDate"], "2026-10-01");
    assert_eq!(v["childrenTruncated"], true);
    assert_eq!(v["relations"][3]["kind"], "blocked_by");
    assert_eq!(v["relations"][3]["issue"]["state"]["type"], "started");
    assert_eq!(v["comments"][0]["createdAt"], "2026-09-02T10:00:00.000Z");
}

#[test]
fn not_found_is_an_api_error() {
    let body = r#"{"data":null,"errors":[{"message":"Entity not found: Issue",
          "extensions":{"code":"INVALID_INPUT","userPresentableMessage":"Could not find referenced Issue."}}]}"#;
    assert_eq!(
        interpret_response::<IssueDetailData>(200, body).unwrap_err(),
        LinearError::Api("Could not find referenced Issue.".into())
    );
}

#[test]
fn query_uses_bounded_page_sizes() {
    let q = issue_detail_query();
    assert!(q.contains("children(first: 50)"));
    assert!(q.contains("comments(first: 20, orderBy: createdAt)"));
    assert!(q.contains("inverseRelations(first: 25)"));
}
