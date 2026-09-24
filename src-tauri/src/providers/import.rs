//! Importación de ítems del proveedor como tareas de Nodal.

use std::path::Path;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::db::{rows, DbError};
use crate::domain::{ExtKind, ExternalState, RepoRule, RuleKind, SourceLink, Task};

use super::plan::{extract_acceptance, plan_path, render_plan, write_plan};
use super::state_map::{propose_pull, pull_status};
use super::{iso_from_ms, store, ExternalItem, ImportQuery, OPEN_KINDS};
use crate::util::new_id;

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

/// Regla de proyecto del link para ese proyecto del proveedor.
pub fn project_rule<'a>(link: &'a SourceLink, project_id: Option<&str>) -> Option<&'a RepoRule> {
    let project_id = project_id?;
    link.repo_rules.iter().find(|r| r.is_project() && r.value == project_id)
}

/// Repo sugerido. Precedencia: la regla del proyecto del ítem > la primera regla cuyo label
/// tenga el ítem (sin distinguir mayúsculas) > el repo por defecto del link.
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

/// `suggest_repo` de un ítem.
pub fn suggest_repo_for(link: &SourceLink, item: &ExternalItem) -> Option<String> {
    suggest_repo(link, item.project().as_ref().map(|p| p.id.as_str()), &item.labels)
}

// ---------- Reglas de proyecto: backfill ----------

/// El backfill trae, además de las abiertas, las cerradas hace menos de esto.
pub const BACKFILL_CLOSED_DAYS: u32 = 14;
/// Tope de páginas del backfill (25 por página).
pub const BACKFILL_MAX_PAGES: usize = 40;

/// Consulta de una regla de proyecto: el backfill (abiertas + cerradas hace menos de
/// `BACKFILL_CLOSED_DAYS`) o el auto-import (abiertas creadas después de la regla).
pub fn rule_query(link: &SourceLink, rule: &RepoRule, backfill: bool) -> ImportQuery {
    ImportQuery {
        project_id: Some(rule.value.clone()),
        closed_within_days: backfill.then_some(BACKFILL_CLOSED_DAYS),
        created_after: (!backfill).then_some(rule.created_at),
        ..ImportQuery::new(link.scope.clone())
    }
}

/// Control local del filtro del backfill (el proveedor ya filtra; esto cubre una respuesta
/// más amplia): abiertas, o completadas/canceladas hace menos de `BACKFILL_CLOSED_DAYS`. Una
/// cerrada sin fecha de cierre se deja pasar (el proveedor la eligió por su fecha).
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

/// Espejo de `RulePreview` en `api.ts`.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RulePreview {
    /// Ítems que se importarían al repo de la regla.
    pub count: usize,
    /// Ya importados en el repo de la regla.
    pub already_imported: usize,
    /// Ya importados en otro repo (u otro proyecto): no se mueven.
    pub in_other_repos: usize,
}

/// Cómo se reparte el backfill de una regla.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BackfillPlan {
    /// External ids a importar.
    pub to_import: Vec<String>,
    /// Tareas (ids) ya importadas en el repo de la regla.
    pub here: Vec<String>,
    /// Ya importadas en otro repo: (external id, motivo).
    pub elsewhere: Vec<Skipped>,
    /// Desvinculadas a mano (lápida): no se traen.
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

/// Filas del listado: sugiere repo y marca las ya importadas.
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
    // Llegó por regla de proyecto si su proyecto tiene regla y el repo elegido es el de ella.
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
                description: None,
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
                description: None,
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
            repo_rules: vec![RepoRule::label("Docs", "r-docs")],
            state_map: map,
            auto_import: false,
            created_at: 1,
            last_synced_at: None,
            last_sync_error: None,
            pending_state_changes: None,
        };
        insert_source_link(conn, &link).unwrap();
        link
    }

    pub fn tmp_dir() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("nodal-import-{}", new_id('x', crate::util::now_ms())))
    }

    #[test]
    fn suggest_repo_by_rule_then_default() {
        let db = open_in_memory().unwrap();
        let conn = db.lock().unwrap();
        let mut link = seed(&conn);
        assert_eq!(suggest_repo(&link, None, &["docs".into()]).as_deref(), Some("r-docs"));
        assert_eq!(suggest_repo(&link, None, &["frontend".into()]), None);
        link.default_repo_id = Some("r-web".into());
        assert_eq!(suggest_repo(&link, None, &["frontend".into()]).as_deref(), Some("r-web"));
    }

    /// Un ítem del proyecto `proj` (id `proj-{proj}`) del proveedor.
    pub fn in_project(mut it: ExternalItem, proj: &str) -> ExternalItem {
        it.scopes.retain(|s| s.kind != "project");
        it.scopes.push(ScopeRef { kind: "project".into(), id: format!("proj-{proj}"), name: proj.into() });
        it
    }

    #[test]
    fn project_rule_beats_label_rule_beats_default() {
        let db = open_in_memory().unwrap();
        let conn = db.lock().unwrap();
        let mut link = seed(&conn);
        link.default_repo_id = Some("r-web".into());
        link.repo_rules.push(RepoRule::project("proj-site", "Site", "r-web", 1));
        link.repo_rules.push(RepoRule::project("proj-guides", "Guides", "r-docs", 1));
        let labels = vec!["docs".to_string()];
        // Proyecto con regla gana al label.
        assert_eq!(suggest_repo(&link, Some("proj-site"), &labels).as_deref(), Some("r-web"));
        assert_eq!(suggest_repo(&link, Some("proj-guides"), &[]).as_deref(), Some("r-docs"));
        // Proyecto sin regla: label, y si no, default.
        assert_eq!(suggest_repo(&link, Some("proj-other"), &labels).as_deref(), Some("r-docs"));
        assert_eq!(suggest_repo(&link, Some("proj-other"), &[]).as_deref(), Some("r-web"));
        // Una regla de label cuyo valor coincide con un id de proyecto no es de proyecto.
        link.repo_rules.push(RepoRule::label("proj-x", "r-docs"));
        assert_eq!(suggest_repo(&link, Some("proj-x"), &[]).as_deref(), Some("r-web"));
        assert_eq!(suggest_repo_for(&link, &in_project(item(1, None), "site")).as_deref(), Some("r-web"));
    }

    #[test]
    fn legacy_rules_json_still_reads() {
        let db = open_in_memory().unwrap();
        let conn = db.lock().unwrap();
        let link = seed(&conn);
        conn.execute(
            r#"UPDATE source_links SET repo_rules_json = '[{"label":"Docs","repoId":"r-docs"},{"label":"api","repoId":"r-web"}]' WHERE id = ?1"#,
            [&link.id],
        )
        .unwrap();
        let a = store::get_link(&conn, &link.id).unwrap();
        let b = store::get_link(&conn, &link.id).unwrap();
        assert_eq!(a.repo_rules, b.repo_rules, "ids estables entre lecturas");
        let r = &a.repo_rules[1];
        assert_eq!((r.kind, r.value.as_str(), r.name.as_str(), r.repo_id.as_str()), (RuleKind::Label, "api", "api", "r-web"));
        assert_eq!((r.id.as_str(), r.created_at), ("rule-1", link.created_at));
        assert_eq!(suggest_repo(&a, None, &["DOCS".into()]).as_deref(), Some("r-docs"));
        // Guardado y releído: formato nuevo, mismos datos.
        store::save_link(&conn, &a).unwrap();
        let json: String =
            conn.query_row("SELECT repo_rules_json FROM source_links WHERE id = ?1", [&link.id], |r| r.get(0)).unwrap();
        assert!(json.contains(r#""kind":"label""#) && json.contains(r#""value":"api""#), "{json}");
        assert_eq!(store::get_link(&conn, &link.id).unwrap().repo_rules, a.repo_rules);

        // Un tipo desconocido (versión futura) no rompe la lectura ni rutea.
        conn.execute(
            r#"UPDATE source_links SET repo_rules_json = '[{"kind":"milestone","value":"m1","repoId":"r-docs"}]' WHERE id = ?1"#,
            [&link.id],
        )
        .unwrap();
        let l = store::get_link(&conn, &link.id).unwrap();
        assert_eq!(l.repo_rules[0].kind, RuleKind::Unknown);
        assert_eq!(suggest_repo(&l, Some("m1"), &["m1".into()]), None);
    }

    #[test]
    fn backfill_keeps_open_and_recently_closed() {
        let now = 1_790_000_000_000; // 2026-09-21
        let day = 86_400_000;
        let closed = |kind: ExtKind, ago_days: Option<i64>| {
            let mut it = item(1, None);
            it.state.kind = kind;
            it.closed_at = ago_days.map(|d| iso_from_ms(now - d * day));
            it
        };
        assert!(backfill_keeps(&closed(ExtKind::Backlog, None), now));
        assert!(backfill_keeps(&closed(ExtKind::Triage, None), now));
        assert!(backfill_keeps(&closed(ExtKind::Completed, Some(3)), now));
        assert!(backfill_keeps(&closed(ExtKind::Canceled, Some(13)), now));
        assert!(!backfill_keeps(&closed(ExtKind::Completed, Some(15)), now));
        assert!(!backfill_keeps(&closed(ExtKind::Canceled, Some(40)), now));
        assert!(backfill_keeps(&closed(ExtKind::Completed, None), now), "sin fecha: confía en el proveedor");
        assert!(!backfill_keeps(&closed(ExtKind::Unknown, None), now));
    }

    #[test]
    fn backfill_plan_splits_new_here_elsewhere_and_unlinked() {
        let db = open_in_memory().unwrap();
        let mut conn = db.lock().unwrap();
        let mut link = seed(&conn);
        let rule = RepoRule::project("proj-guides", "Guides", "r-docs", 5);
        link.repo_rules.push(rule.clone());
        let dir = tmp_dir();
        let items: Vec<_> = (1..=5).map(|n| in_project(item(n, None), "guides")).collect();
        // 1 ya en el repo de la regla, 2 en otro repo, 3 desvinculada, 4 y 5 nuevas.
        let r = import_items(
            &mut conn,
            &dir,
            &link,
            vec![(items[0].clone(), "r-docs".into()), (items[1].clone(), "r-web".into()), (items[2].clone(), "r-web".into())],
            2,
        )
        .unwrap();
        store::unlink_task(&conn, &r.imported[2].id, 3).unwrap();
        let mut dup = items.clone();
        dup.push(items[4].clone());
        let plan = plan_backfill(&conn, &link, &rule, &dup).unwrap();
        assert_eq!(plan.to_import, vec!["uuid-4", "uuid-5"]);
        assert_eq!(plan.here, vec![r.imported[0].id.clone()]);
        assert_eq!(plan.unlinked, 1);
        assert_eq!(plan.elsewhere.len(), 1);
        assert!(plan.elsewhere[0].reason.contains("ENG-2 is already imported in r-web"), "{:?}", plan.elsewhere);
        assert_eq!(plan.preview(), RulePreview { count: 2, already_imported: 1, in_other_repos: 1 });
        // Importada al repo de su regla: queda asociada a la regla y con el proyecto.
        let t = &r.imported[0];
        let src = t.source.as_ref().unwrap();
        assert_eq!(src.rule_id.as_deref(), Some("rule-proj-guides"));
        assert_eq!(src.project.as_ref().map(|p| p.id.as_str()), Some("proj-guides"));
        assert_eq!(r.imported[1].source.as_ref().unwrap().rule_id, None, "otro repo: no llegó por la regla");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn rule_queries_backfill_vs_auto_import() {
        let db = open_in_memory().unwrap();
        let conn = db.lock().unwrap();
        let link = seed(&conn);
        let rule = RepoRule::project("proj-guides", "Guides", "r-docs", 77);
        let b = rule_query(&link, &rule, true);
        assert_eq!((b.project_id.as_deref(), b.closed_within_days, b.created_after), (Some("proj-guides"), Some(14), None));
        assert_eq!(b.state_kinds, OPEN_KINDS.to_vec());
        let a = rule_query(&link, &rule, false);
        assert_eq!((a.closed_within_days, a.created_after), (None, Some(77)));
        assert_eq!(a.scope, link.scope);
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
