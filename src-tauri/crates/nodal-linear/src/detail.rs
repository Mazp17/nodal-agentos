//! Issue detail (side panel): description, sub-issues, relations, labels and
//! comments. Pure (no network) so it can be tested with fixtures.
//!
//! Fields verified against the official SDK schema
//! (github.com/linear/linear, packages/sdk/src/schema.graphql):
//! `Issue.{description, estimate, dueDate, createdAt, creator, parent, labels,
//! children, relations, inverseRelations, comments}`, `IssueRelation.{type, issue,
//! relatedIssue}` and `IssueRelationType = blocks | duplicate | related | similar`.
//! There is no "blocked_by" type: it is a `blocks` seen from `inverseRelations`.

use super::model::{PageInfo, UserRef};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Caps for each nested connection. Linear scores complexity by multiplying by
/// `first`; with these values the query stays at a few hundred points (limit 10k).
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
    /// Only `duplicate` can have it true: the other issue is a duplicate of this one.
    pub inverse: bool,
    pub issue: IssueRef,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub id: String,
    pub body: String,
    /// Author user or bot; `None` if Linear exposes neither (e.g. integrations).
    pub author: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDetail {
    pub id: String,
    pub identifier: String,
    /// Markdown as stored by Linear.
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
    /// First page (`COMMENTS_FIRST`) in Linear's default order, re-sorted in ascending
    /// chronological order. Whether it is the most recent page is not verified.
    pub comments: Vec<Comment>,
    pub comments_truncated: bool,
}

// ---- Raw response shape ----

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

/// `inverse` = the relation comes from `inverseRelations` (the other issue is the subject).
/// `similar` (Linear's automatic suggestions) and unknown types are dropped.
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
                // "related" can come from both sides; a single row per (type, issue).
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
        // ISO 8601 in UTC: lexicographic order is chronological order.
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

/// `issue(id:)` accepts both the UUID and the identifier ("ACME-8").
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
mod tests;
