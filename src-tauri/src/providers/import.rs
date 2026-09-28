//! Importing provider items as Nodal tasks.

use std::path::Path;

use rusqlite::Connection;

use crate::db::{rows, DbError};
use crate::domain::{RepoRule, SourceLink, Task};

use super::plan::{extract_acceptance, plan_path, render_plan, write_plan};
use super::state_map::{propose_pull, pull_status};
use super::{store, ExternalItem};
use crate::util::new_id;

/// Moved to `nodal_domain::sources::routing`; re-exported so current uses don't break.
pub use nodal_domain::sources::routing::*;

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
