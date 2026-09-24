//! Importación de ítems del proveedor como tareas de Nodal.

use std::path::Path;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::db::DbError;
use crate::domain::{ExternalState, SourceLink, Task};

use super::plan::{extract_acceptance, plan_path, render_plan, write_plan};
use super::state_map::{propose_pull, pull_status};
use super::{new_id, store, ExternalItem};

/// Espejo de `ImportableItem` en `api.ts`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportableItem {
    pub external_id: String,
    pub identifier: String,
    pub title: String,
    pub url: String,
    pub state: ExternalState,
    pub labels: Vec<String>,
    /// Por `repo_rules` o `default_repo_id`.
    pub suggested_repo_id: Option<String>,
    /// Si ya está importada, su tarea.
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

/// Espejo de `ImportResult` en `api.ts`.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub imported: Vec<Task>,
    pub skipped: Vec<Skipped>,
}

/// Repo sugerido: la primera regla cuyo label tenga el ítem (sin distinguir mayúsculas);
/// si ninguna aplica, el repo por defecto del link.
pub fn suggest_repo(link: &SourceLink, labels: &[String]) -> Option<String> {
    link.repo_rules
        .iter()
        .find(|r| labels.iter().any(|l| l.trim().eq_ignore_ascii_case(r.label.trim())))
        .map(|r| r.repo_id.clone())
        .or_else(|| link.default_repo_id.clone())
}

/// Filas del listado: sugiere repo y marca las ya importadas.
pub fn importable_rows(conn: &Connection, link: &SourceLink, items: Vec<ExternalItem>) -> Result<Vec<ImportableItem>, DbError> {
    items
        .into_iter()
        .map(|i| {
            let task_id = store::task_by_external(conn, &link.provider, &i.external_id)?.map(|t| t.id);
            Ok(ImportableItem {
                suggested_repo_id: suggest_repo(link, &i.labels),
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

/// Importa `items` (ítems completos, con su repo ya elegido) en el proyecto del link. Cada
/// ítem va en su propia transacción: uno que falla se reporta y no frena al resto.
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
    // Un estado sin mapear no tiene estado Nodal: la tarea nueva arranca con la propuesta.
    let status = pull_status(&link.state_map, &item.state).unwrap_or_else(|| propose_pull(&item.state));
    let acceptance = item.description_md.as_deref().map(extract_acceptance).unwrap_or_default();

    let tx = conn.transaction()?;
    let task = store::insert_imported(
        &tx,
        store::NewImported {
            id: new_id(now),
            project_id: &link.project_id,
            repo_id,
            link,
            item,
            status,
            acceptance,
            now,
        },
    )?;
    // El archivo va antes del commit: si no se puede escribir, la tarea no queda a medias.
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
pub mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::db::rows::{insert_project, insert_repo, insert_source_link};
    use crate::domain::*;
    use crate::providers::plan::tests::item;
    use crate::providers::state_map::{propose, tests::team_states};

    pub fn seed(conn: &Connection) -> SourceLink {
        insert_project(
            conn,
            &Project {
                id: "p1".into(),
                name: "Acme Web".into(),
                key: "WEB".into(),
                next_task_number: 1,
                color: "#fff".into(),
                default_executor: None,
                reviewer: None,
                created_at: 1,
                archived_at: None,
            },
        )
        .unwrap();
        insert_project(
            conn,
            &Project {
                id: "p2".into(),
                name: "Acme Ops".into(),
                key: "OPS".into(),
                next_task_number: 1,
                color: "#fff".into(),
                default_executor: None,
                reviewer: None,
                created_at: 1,
                archived_at: None,
            },
        )
        .unwrap();
        for (id, project) in [("r-web", "p1"), ("r-docs", "p1"), ("r-ops", "p2")] {
            insert_repo(
                conn,
                &Repo {
                    id: id.into(),
                    project_id: project.into(),
                    path: format!("/tmp/acme-{id}"),
                    name: id.into(),
                    launch: LaunchOptions::default(),
                    default_executor: None,
                    default_isolation: Isolation::Worktree,
                    default_finish: Finish::Pr,
                    default_review: true,
                    reviewer: None,
                    position: 0,
                    created_at: 1,
                },
            )
            .unwrap();
        }
        let mut map = propose(&team_states());
        map.confirmed_at = Some(1);
        let link = SourceLink {
            id: "l1".into(),
            project_id: "p1".into(),
            provider: "fake".into(),
            scope: ScopeRef { kind: "team".into(), id: "team-eng".into(), name: "Engineering".into() },
            default_repo_id: None,
            repo_rules: vec![RepoRule { label: "Docs".into(), repo_id: "r-docs".into() }],
            state_map: map,
            auto_import: false,
            created_at: 1,
        };
        insert_source_link(conn, &link).unwrap();
        link
    }

    pub fn tmp_dir() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("nodal-import-{}", new_id(crate::providers::now_ms())))
    }

    #[test]
    fn suggest_repo_by_rule_then_default() {
        let db = open_in_memory().unwrap();
        let conn = db.lock().unwrap();
        let mut link = seed(&conn);
        assert_eq!(suggest_repo(&link, &["docs".into()]).as_deref(), Some("r-docs"));
        assert_eq!(suggest_repo(&link, &["frontend".into()]), None);
        link.default_repo_id = Some("r-web".into());
        assert_eq!(suggest_repo(&link, &["frontend".into()]).as_deref(), Some("r-web"));
    }

    #[test]
    fn imports_with_plan_criteria_and_status() {
        let db = open_in_memory().unwrap();
        let mut conn = db.lock().unwrap();
        let link = seed(&conn);
        let dir = tmp_dir();
        let mut it = item(142, Some("Texto.\n\n## Acceptance\n- Logo nuevo\n- Tests verdes\n"));
        it.labels = vec!["frontend".into()];
        it.priority = Priority::High;
        it.state = team_states().into_iter().find(|s| s.id == "s-review").unwrap();

        let r = import_items(&mut conn, &dir, &link, vec![(it.clone(), "r-web".into())], 10).unwrap();
        assert!(r.skipped.is_empty(), "{:?}", r.skipped);
        let t = &r.imported[0];
        assert_eq!(t.number, 1);
        assert_eq!(t.status, TaskStatus::InReview);
        assert_eq!(t.priority, Priority::High);
        assert_eq!(t.labels, vec!["frontend"]);
        assert_eq!(t.acceptance, vec!["Logo nuevo", "Tests verdes"]);
        let src = t.source.as_ref().unwrap();
        assert_eq!(src.identifier, "ENG-142");
        assert_eq!(src.link_id.as_deref(), Some("l1"));
        assert_eq!(src.external_state.as_ref().unwrap().id, "s-review");
        let plan = std::fs::read_to_string(plan_path(&dir, &t.id)).unwrap();
        assert!(plan.starts_with("# ENG-142 · Issue 142\n"));
        assert_eq!(crate::db::rows::get_task(&conn, &t.id).unwrap().as_ref(), Some(t));

        // Segunda vez: ya importada. Repo de otro proyecto: rechazado.
        let mut other = item(7, None);
        other.external_id = "uuid-7".into();
        let r = import_items(&mut conn, &dir, &link, vec![(it, "r-web".into()), (other, "r-ops".into())], 11).unwrap();
        assert!(r.imported.is_empty());
        assert!(r.skipped[0].reason.contains("already imported"));
        assert!(r.skipped[1].reason.contains("another project"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn listing_marks_imported_and_suggests_repo() {
        let db = open_in_memory().unwrap();
        let mut conn = db.lock().unwrap();
        let link = seed(&conn);
        let dir = tmp_dir();
        let a = item(1, None);
        let mut b = item(2, None);
        b.labels = vec!["DOCS".into()];
        let r = import_items(&mut conn, &dir, &link, vec![(a.clone(), "r-web".into())], 1).unwrap();
        let rows = importable_rows(&conn, &link, vec![a, b]).unwrap();
        assert_eq!(rows[0].task_id.as_deref(), Some(r.imported[0].id.as_str()));
        assert_eq!(rows[1].task_id, None);
        assert_eq!(rows[1].suggested_repo_id.as_deref(), Some("r-docs"));
        std::fs::remove_dir_all(dir).ok();
    }
}
