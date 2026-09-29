//! Repo routing for imported items: which repo a project or label rule sends an item to, and
//! the backfill filter/query building. Reading and writing tasks lives in nodal-app.

use serde::{Deserialize, Serialize};

use crate::model::providers::{ExternalItem, ImportQuery, OPEN_KINDS};
use crate::model::{ExtKind, ExternalState, RepoRule, RuleKind, SourceLink, Task};

use super::iso_from_ms;

/// The link's project rule for that provider project.
pub fn project_rule<'a>(link: &'a SourceLink, project_id: Option<&str>) -> Option<&'a RepoRule> {
    let project_id = project_id?;
    link.repo_rules
        .iter()
        .find(|r| r.is_project() && r.value == project_id)
}

/// Suggested repo. Precedence: the item's project rule > the first rule whose label the item
/// has (case-insensitive) > the link's default repo.
pub fn suggest_repo(
    link: &SourceLink,
    project_id: Option<&str>,
    labels: &[String],
) -> Option<String> {
    project_rule(link, project_id)
        .or_else(|| {
            link.repo_rules.iter().find(|r| {
                r.kind == RuleKind::Label
                    && labels
                        .iter()
                        .any(|l| l.trim().eq_ignore_ascii_case(r.value.trim()))
            })
        })
        .map(|r| r.repo_id.clone())
        .or_else(|| link.default_repo_id.clone())
}

/// `suggest_repo` for an item.
pub fn suggest_repo_for(link: &SourceLink, item: &ExternalItem) -> Option<String> {
    suggest_repo(
        link,
        item.project().as_ref().map(|p| p.id.as_str()),
        &item.labels,
    )
}

// ---------- Project rules: backfill ----------

/// Besides the open ones, the backfill brings those closed less than this ago.
pub const BACKFILL_CLOSED_DAYS: u32 = 14;
/// Backfill page cap (25 per page).
pub const BACKFILL_MAX_PAGES: usize = 40;

/// Query for a project rule: the backfill (open + closed less than `BACKFILL_CLOSED_DAYS`
/// ago) or auto-import (open ones created after the rule).
pub fn rule_query(link: &SourceLink, rule: &RepoRule, backfill: bool) -> ImportQuery {
    ImportQuery {
        project_id: Some(rule.value.clone()),
        closed_within_days: backfill.then_some(BACKFILL_CLOSED_DAYS),
        created_after: (!backfill).then_some(rule.created_at),
        ..ImportQuery::new(link.scope.clone())
    }
}

/// Local check of the backfill filter (the provider already filters; this covers a broader
/// response): open, or completed/canceled less than `BACKFILL_CLOSED_DAYS` ago. A closed one
/// with no close date is let through (the provider picked it by its date).
pub fn backfill_keeps(item: &ExternalItem, now: i64) -> bool {
    if OPEN_KINDS.contains(&item.state.kind) {
        return true;
    }
    if !matches!(item.state.kind, ExtKind::Completed | ExtKind::Canceled) {
        return false;
    }
    let since = iso_from_ms(now - i64::from(BACKFILL_CLOSED_DAYS) * 86_400_000);
    item.closed_at.as_deref().is_none_or(|c| c > since.as_str())
}

/// Mirror of `ImportableItem` in `api.ts`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportableItem {
    pub external_id: String,
    pub identifier: String,
    pub title: String,
    pub url: String,
    pub state: ExternalState,
    pub labels: Vec<String>,
    /// From `repo_rules` or `default_repo_id`.
    pub suggested_repo_id: Option<String>,
    /// If already imported, its task.
    pub task_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    pub external_id: String,
    pub repo_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skipped {
    pub external_id: String,
    pub reason: String,
}

/// Mirror of `ImportResult` in `api.ts`.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub imported: Vec<Task>,
    pub skipped: Vec<Skipped>,
}

/// Mirror of `RulePreview` in `api.ts`.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RulePreview {
    /// Items that would be imported into the rule's repo.
    pub count: usize,
    /// Already imported into the rule's repo.
    pub already_imported: usize,
    /// Already imported into another repo (or another project): not moved.
    pub in_other_repos: usize,
}

/// How a rule's backfill is split.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BackfillPlan {
    /// External ids to import.
    pub to_import: Vec<String>,
    /// Tasks (ids) already imported into the rule's repo.
    pub here: Vec<String>,
    /// Already imported into another repo: (external id, reason).
    pub elsewhere: Vec<Skipped>,
    /// Unlinked by hand (tombstone): not brought in.
    pub unlinked: usize,
}

impl BackfillPlan {
    pub fn preview(&self) -> RulePreview {
        RulePreview {
            count: self.to_import.len(),
            already_imported: self.here.len(),
            in_other_repos: self.elsewhere.len(),
        }
    }
}

#[cfg(test)]
mod tests;
