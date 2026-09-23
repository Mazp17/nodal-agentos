//! Detalle de una issue (panel lateral): descripción, sub-issues, relaciones, labels y
//! comentarios. Puro (sin red) para testearlo con fixtures.
//!
//! Campos verificados contra el schema oficial del SDK
//! (github.com/linear/linear, packages/sdk/src/schema.graphql):
//! `Issue.{description, estimate, dueDate, createdAt, creator, parent, labels,
//! children, relations, inverseRelations, comments}`, `IssueRelation.{type, issue,
//! relatedIssue}` e `IssueRelationType = blocks | duplicate | related | similar`.
//! No existe un tipo "blocked_by": es un `blocks` visto desde `inverseRelations`.

use super::model::{PageInfo, UserRef};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Topes de cada conexión anidada. Linear puntúa la complejidad multiplicando por
/// `first`; con estos valores la query queda en unos cientos de puntos (límite 10k).
pub const CHILDREN_FIRST: u32 = 50;
pub const RELATIONS_FIRST: u32 = 25;
pub const LABELS_FIRST: u32 = 20;
pub const COMMENTS_FIRST: u32 = 20;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateRef {
    pub name: String,
    #[serde(rename = "type")]
    pub state_type: String,
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueRef {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub state: StateRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubIssue {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub state: StateRef,
    pub assignee: Option<UserRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Label {
    pub id: String,
    pub name: String,
    pub color: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    Blocks,
    BlockedBy,
    Related,
    Duplicate,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Relation {
    pub kind: RelationKind,
    /// Sólo `duplicate` puede tenerlo en true: la otra issue es duplicado de ésta.
    pub inverse: bool,
    pub issue: IssueRef,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub id: String,
    pub body: String,
    /// Usuario o bot autor; `None` si Linear no expone ninguno (p. ej. integraciones).
    pub author: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDetail {
    pub id: String,
    pub identifier: String,
    /// Markdown tal cual lo guarda Linear.
    pub description: Option<String>,
    pub parent: Option<IssueRef>,
    pub children: Vec<SubIssue>,
    pub children_truncated: bool,
    pub relations: Vec<Relation>,
    pub labels: Vec<Label>,
    pub estimate: Option<f64>,
    /// `TimelessDate` ("YYYY-MM-DD").
    pub due_date: Option<String>,
    pub created_at: String,
    pub creator: Option<UserRef>,
    /// Primera página (`COMMENTS_FIRST`) en el orden por defecto de Linear, reordenada
    /// cronológicamente ascendente. Que sea la página más reciente no está verificado.
    pub comments: Vec<Comment>,
    pub comments_truncated: bool,
}

// ---- Forma cruda de la respuesta ----

#[derive(Debug, Deserialize)]
pub struct IssueDetailData {
    issue: RawIssue,
}

#[derive(Debug, Deserialize)]
struct Nodes<T> {
    nodes: Vec<T>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Paged<T> {
    nodes: Vec<T>,
    page_info: PageInfo,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawIssue {
    id: String,
    identifier: String,
    description: Option<String>,
    estimate: Option<f64>,
    due_date: Option<String>,
    created_at: String,
    creator: Option<UserRef>,
    parent: Option<IssueRef>,
    labels: Nodes<Label>,
    children: Paged<SubIssue>,
    relations: Nodes<RawRelation>,
    inverse_relations: Nodes<RawInverseRelation>,
    comments: Paged<RawComment>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRelation {
    #[serde(rename = "type")]
    rel_type: String,
    related_issue: IssueRef,
}

#[derive(Debug, Deserialize)]
struct RawInverseRelation {
    #[serde(rename = "type")]
    rel_type: String,
    issue: IssueRef,
}

#[derive(Debug, Deserialize)]
struct BotRef {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawComment {
    id: String,
    body: String,
    created_at: String,
    user: Option<UserRef>,
    bot_actor: Option<BotRef>,
}

/// `inverse` = la relación viene de `inverseRelations` (la otra issue es el sujeto).
/// `similar` (sugerencias automáticas de Linear) y tipos desconocidos se descartan.
fn relation_kind(rel_type: &str, inverse: bool) -> Option<RelationKind> {
    match (rel_type, inverse) {
        ("blocks", false) => Some(RelationKind::Blocks),
        ("blocks", true) => Some(RelationKind::BlockedBy),
        ("related", _) => Some(RelationKind::Related),
        ("duplicate", _) => Some(RelationKind::Duplicate),
        _ => None,
    }
}

impl IssueDetailData {
    pub fn into_detail(self) -> IssueDetail {
        let i = self.issue;

        let direct = i
            .relations
            .nodes
            .into_iter()
            .map(|r| (r.rel_type, false, r.related_issue));
        let inverse = i
            .inverse_relations
            .nodes
            .into_iter()
            .map(|r| (r.rel_type, true, r.issue));
        let mut seen = HashSet::new();
        let relations = direct
            .chain(inverse)
            .filter_map(|(t, inv, issue)| {
                let kind = relation_kind(&t, inv)?;
                // "related" puede venir de ambos lados; una sola fila por (tipo, issue).
                let inverse = inv && kind == RelationKind::Duplicate;
                seen.insert((kind, inverse, issue.id.clone()))
                    .then_some(Relation { kind, inverse, issue })
            })
            .collect();

        let mut comments: Vec<Comment> = i
            .comments
            .nodes
            .into_iter()
            .map(|c| Comment {
                id: c.id,
                body: c.body,
                author: c
                    .user
                    .map(|u| if u.display_name.is_empty() { u.name } else { u.display_name })
                    .or_else(|| c.bot_actor.and_then(|b| b.name)),
                created_at: c.created_at,
            })
            .collect();
        // ISO 8601 en UTC: el orden lexicográfico es el cronológico.
        comments.sort_by(|a, b| a.created_at.cmp(&b.created_at));

        IssueDetail {
            id: i.id,
            identifier: i.identifier,
            description: i.description.filter(|d| !d.trim().is_empty()),
            parent: i.parent,
            children: i.children.nodes,
            children_truncated: i.children.page_info.has_next_page,
            relations,
            labels: i.labels.nodes,
            estimate: i.estimate,
            due_date: i.due_date,
            created_at: i.created_at,
            creator: i.creator,
            comments,
            comments_truncated: i.comments.page_info.has_next_page,
        }
    }
}

/// `issue(id:)` acepta tanto el UUID como el identifier ("ACME-8").
pub fn issue_detail_query() -> String {
    format!(
        "query IssueDetail($id: String!) {{
  issue(id: $id) {{
    id identifier description estimate dueDate createdAt
    creator {{ id name displayName }}
    parent {{ id identifier title state {{ name type color }} }}
    labels(first: {LABELS_FIRST}) {{ nodes {{ id name color }} }}
    children(first: {CHILDREN_FIRST}) {{
      nodes {{ id identifier title state {{ name type color }} assignee {{ id name displayName }} }}
      pageInfo {{ hasNextPage endCursor }}
    }}
    relations(first: {RELATIONS_FIRST}) {{
      nodes {{ type relatedIssue {{ id identifier title state {{ name type color }} }} }}
    }}
    inverseRelations(first: {RELATIONS_FIRST}) {{
      nodes {{ type issue {{ id identifier title state {{ name type color }} }} }}
    }}
    comments(first: {COMMENTS_FIRST}, orderBy: createdAt) {{
      nodes {{ id body createdAt user {{ id name displayName }} botActor {{ name }} }}
      pageInfo {{ hasNextPage endCursor }}
    }}
  }}
}}"
    )
}

#[cfg(test)]
mod tests {
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
}
