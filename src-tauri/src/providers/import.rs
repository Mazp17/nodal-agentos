//! Importing provider items as Nodal tasks.

use std::path::Path;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::db::{rows, DbError};
use crate::domain::{ExtKind, ExternalState, RepoRule, RuleKind, SourceLink, Task};

use super::plan::{extract_acceptance, plan_path, render_plan, write_plan};
use super::state_map::{propose_pull, pull_status};
use super::{iso_from_ms, store, ExternalItem, ImportQuery, OPEN_KINDS};
use crate::util::new_id;

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

/// The link's project rule for that provider project.
pub fn project_rule<'a>(link: &'a SourceLink, project_id: Option<&str>) -> Option<&'a RepoRule> {
    let project_id = project_id?;
    link.repo_rules.iter().find(|r| r.is_project() && r.value == project_id)
}

/// Suggested repo. Precedence: the item's project rule > the first rule whose label the item
/// has (case-insensitive) > the link's default repo.
pub fn suggest_repo(link: &SourceLink, project_id: Option<&str>, labels: &[String]) -> Option<String> {
    project_rule(link, project_id)
        .or_else(|| {
            link.repo_rules.iter().find(|r| {
                r.kind == RuleKind::Label && labels.iter().any(|l| l.trim().eq_ignore_ascii_case(r.value.trim()))
            })
        })
        .map(|r| r.repo_id.clone())
        .or_else(|| link.default_repo_id.clone())
}

/// `suggest_repo` for an item.
pub fn suggest_repo_for(link: &SourceLink, item: &ExternalItem) -> Option<String> {
    suggest_repo(link, item.project().as_ref().map(|p| p.id.as_str()), &item.labels)
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

pub fn plan_backfill(
    conn: &Connection,
    link: &SourceLink,
    rule: &RepoRule,
    items: &[ExternalItem],
) -> Result<BackfillPlan, DbError> {
    let mut plan = BackfillPlan::default();
    let mut seen = std::collections::HashSet::new();
    for i in items {
        if !seen.insert(i.external_id.as_str()) {
            continue;
        }
        match store::task_by_external(conn, &link.provider, &i.external_id)? {
            Some(t) if t.project_id == link.project_id && t.repo_id == rule.repo_id => plan.here.push(t.id),
            Some(t) => {
                let where_ = if t.project_id == link.project_id {
                    match rows::get_repo(conn, &t.repo_id)? {
                        Some(r) => format!("in {}", r.name),
                        None => "in another repo".to_string(),
                    }
                } else {
                    "in another project".to_string()
                };
                plan.elsewhere.push(Skipped {
                    external_id: i.external_id.clone(),
                    reason: format!("{} is already imported {where_}; left where it is.", i.identifier),
                });
            }
            None if store::is_unlinked(conn, &link.provider, &i.external_id)? => plan.unlinked += 1,
            None => plan.to_import.push(i.external_id.clone()),
        }
    }
    Ok(plan)
}

/// Listing rows: suggests a repo and marks the ones already imported.
pub fn importable_rows(conn: &Connection, link: &SourceLink, items: Vec<ExternalItem>) -> Result<Vec<ImportableItem>, DbError> {
    items
        .into_iter()
        .map(|i| {
            let task_id = store::task_by_external(conn, &link.provider, &i.external_id)?.map(|t| t.id);
            Ok(ImportableItem {
                suggested_repo_id: suggest_repo_for(link, &i),
                external_id: i.external_id,
                identifier: i.identifier,
                title: i.title,
                url: i.url,
                state: i.state,
                labels: i.labels,
                task_id,
            })
        })
        .collect()
}

/// Imports `items` (full items, with their repo already chosen) into the link's project. Each
/// item goes in its own transaction: one that fails is reported and doesn't stop the rest.
pub fn import_items(
    conn: &mut Connection,
    data_dir: &Path,
    link: &SourceLink,
    items: Vec<(ExternalItem, String)>,
    now: i64,
) -> Result<ImportResult, DbError> {
    let mut out = ImportResult::default();
    for (item, repo_id) in items {
        match import_one(conn, data_dir, link, &item, &repo_id, now) {
            Ok(task) => out.imported.push(task),
            Err(reason) => out.skipped.push(Skipped { external_id: item.external_id, reason: reason.to_string() }),
        }
    }
    Ok(out)
}

fn import_one(
    conn: &mut Connection,
    data_dir: &Path,
    link: &SourceLink,
    item: &ExternalItem,
    repo_id: &str,
    now: i64,
) -> Result<Task, DbError> {
    store::check_repo_in_project(conn, repo_id, &link.project_id)?;
    if let Some(t) = store::task_by_external(conn, &link.provider, &item.external_id)? {
        let where_ = if t.project_id == link.project_id { "" } else { " in another project" };
        return Err(DbError::Invalid(format!("{} is already imported{where_}.", item.identifier)));
    }
    // An unmapped state has no Nodal status: the new task starts with the proposal.
    let status = pull_status(&link.state_map, &item.state).unwrap_or_else(|| propose_pull(&item.state));
    let acceptance = item.description_md.as_deref().map(extract_acceptance).unwrap_or_default();
    // It arrived through a project rule if its project has a rule and the chosen repo is the rule's.
    let rule_id = project_rule(link, item.project().as_ref().map(|p| p.id.as_str()))
        .filter(|r| r.repo_id == repo_id)
        .map(|r| r.id.clone());

    let tx = conn.transaction()?;
    let task = store::insert_imported(
        &tx,
        store::NewImported {
            id: new_id('t', now),
            project_id: &link.project_id,
            repo_id,
            link,
            item,
            status,
            acceptance,
            rule_id,
            now,
        },
    )?;
    // The file goes before the commit: if it can't be written, the task isn't left half-done.
    let path = plan_path(data_dir, &task.id);
    write_plan(&path, &render_plan(item))
        .map_err(|e| DbError::Invalid(format!("Could not write the plan for {}: {e}", item.identifier)))?;
    if let Err(e) = tx.commit() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
        return Err(e.into());
    }
    Ok(task)
}

#[cfg(test)]
pub mod tests;
